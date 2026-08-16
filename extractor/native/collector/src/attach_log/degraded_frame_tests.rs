//! The producer half of the degraded-frame contract.
//!
//! Frame completeness was all-or-nothing across the whole scheduled roster: any
//! domain whose non-tolerated status flags fired refused the entire frame. Three
//! walked live captures carried `FRAME_END`x1, `DOMAIN_GAP`x73 and
//! `FRAME_REFUSED`xN, and not one `DOMAIN_DATA` record -- so `accepted_hours`
//! was zero in every capture the project has ever taken, and no domain fix could
//! be verified end to end because nothing was ever published to verify against.
//!
//! These tests pin the split that ends that, and the guards that keep it honest.
//! The rule is applied to every scheduled domain by one classifier; there is no
//! per-domain branch anywhere in it, and no domain opts in.

use super::*;

const TEST_HOUR: i32 = 60_831_436;

/// The completeness walk gained a producer-outcome ledger when the producer
/// migration landed, and these hand-assembled frames have to answer for it.
///
/// An EMPTY ledger is NOT the faithful lift, which is worth stating because it
/// is the obvious one. Domains 18, 29 and 47 are all in `MIGRATED_OUTCOME_DOMAINS`,
/// and a migrated site records an outcome on its FAILURE path by construction --
/// so a failing migrated domain with no ledger entry means the site was
/// bypassed, which the walk correctly reports as a frame-level
/// `migrated_site_published_outcome` finding and which correctly REFUSES the
/// frame. Passing an empty ledger here would therefore not be "these tests do
/// not care about outcomes"; it would silently convert every per-domain fault
/// in this module into a frame fault and test the opposite of what these tests
/// are named for.
///
/// So the ledger is built to match the frame: every domain the frame shows
/// FAILING gets the failure outcome its producer would have recorded. Clean
/// domains get nothing, exactly as a passing read leaves nothing to diagnose.
///
/// Falsifiability is unchanged -- these tests assert the DEGRADED/REFUSED split
/// and the notice's contents, and breaking the split turns them red. What the
/// ledger removes is a second, unrelated reason for them to be red.
fn outcomes_for(
    failing_domains: &[(u16, u32)],
) -> compact_contract::domain_outcome::DomainOutcomeLedger {
    use compact_contract::domain_outcome::{
        read_predicate, AnswerExtent, ClockPresence, DomainOutcome, DomainOutcomeLedger,
        ReadCompletion, READ_PREDICATE_PAYLOAD_ENCODED,
    };
    let mut ledger = DomainOutcomeLedger::new();
    for (domain_id, status_flags) in failing_domains {
        // The outcome must DERIVE the bits the record publishes, not merely
        // exist. `audit_published_status` raises a frame-level
        // `published_status_matches_outcome` finding when an offender's word
        // carries less than its outcome claims, and that finding refuses the
        // frame -- so a fixture whose ledger and PERF word disagree would test
        // the audit rather than the degraded/refused split these tests name.
        let completion = if status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE != 0 {
            ReadCompletion::DidNotComplete
        } else {
            ReadCompletion::Completed
        };
        let extent = if status_flags & COMPACT_STATUS_PARTIAL_DATA != 0 {
            AnswerExtent::Incomplete
        } else {
            AnswerExtent::Whole
        };
        let clock = if status_flags & COMPACT_STATUS_NO_CLOCK != 0 {
            ClockPresence::Unreadable
        } else {
            ClockPresence::Readable
        };
        ledger.record(
            DomainOutcome::failed(
                *domain_id,
                read_predicate(READ_PREDICATE_PAYLOAD_ENCODED)
                    .expect("payload_encoded is a read predicate"),
                completion,
                extent,
                "test fixture: this domain's read did not answer in whole",
            )
            .with_clock(clock),
        );
    }
    ledger
}

/// For a frame with no failing domain at all: nothing failed, so nothing was
/// diagnosed, and the empty ledger is the honest state.
fn no_outcomes() -> compact_contract::domain_outcome::DomainOutcomeLedger {
    outcomes_for(&[])
}

fn perf_payload_with_status(status_flags: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 40];
    payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
    payload
}

fn frame_end_payload() -> Vec<u8> {
    vec![0u8; 32]
}

/// Assemble a frame exactly as the scheduler does: one PERF record per domain,
/// then a single trailing `FRAME_END`. Returns the bytes and the sequence the
/// next record would take.
fn assembled_frame(perf_domains: &[(u16, u32)]) -> (Vec<u8>, u64) {
    let mut records = Vec::new();
    let mut sequence = 1u64;
    for (domain_id, status_flags) in perf_domains {
        append_compact_record(
            &mut records,
            sequence,
            TEST_HOUR,
            *domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_status(*status_flags),
        );
        sequence += 1;
    }
    append_compact_record(
        &mut records,
        sequence,
        TEST_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(),
    );
    sequence += 1;
    (records, sequence)
}

/// Walk a frame and return `(sequence, domain_id, record_type, payload)` for
/// every record, so a test can assert on the published bytes rather than on the
/// producer's intent.
fn walk(records: &[u8]) -> Vec<(u64, u16, u16, Vec<u8>)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..])
            .expect("test frame must stay well framed");
        let end = offset + header.record_len();
        let payload = records[offset + compact_contract::COMPACT_RECORD_HEADER_LEN..end].to_vec();
        out.push((
            header.sequence,
            header.domain_id,
            header.record_type,
            payload,
        ));
        offset = end;
    }
    out
}

/// THE producer deliverable: a frame whose only objections are per-domain
/// PUBLISHES, carrying a notice that names those domains and nothing else.
///
/// Domains 29 and 47 have no `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry, so
/// their bits are genuine untolerated defects -- the shape that refused every
/// live frame.
#[test]
fn a_frame_whose_only_faults_are_per_domain_publishes_with_a_notice_naming_them() {
    let (mut records, mut next_sequence) = assembled_frame(&[
        (18, 0),
        (29, COMPACT_STATUS_PARTIAL_DATA),
        (47, COMPACT_STATUS_SOURCE_UNAVAILABLE),
    ]);
    let findings = compact_frame_completeness_findings(
        &records,
        TEST_HOUR,
        &outcomes_for(&[
            (29, COMPACT_STATUS_PARTIAL_DATA),
            (47, COMPACT_STATUS_SOURCE_UNAVAILABLE),
        ]),
    );
    assert_eq!(
        findings.len(),
        2,
        "both failing domains must object, and the clean one must not; findings: {:?}",
        findings.iter().map(|f| f.text.as_str()).collect::<Vec<_>>()
    );

    let FrameCompletenessVerdict::Degraded(domains) = classify_frame_completeness(&findings) else {
        panic!("per-domain faults alone must not refuse the frame");
    };
    assert_eq!(
        domains
            .iter()
            .map(|domain| (domain.domain_id, domain.non_tolerated_flags))
            .collect::<Vec<_>>(),
        vec![
            (29, COMPACT_STATUS_PARTIAL_DATA),
            (47, COMPACT_STATUS_SOURCE_UNAVAILABLE)
        ]
    );

    let frame_end_sequence_before = walk(&records).last().expect("frame has records").0;
    insert_frame_degraded_notice_before_frame_end(
        &mut records,
        &mut next_sequence,
        TEST_HOUR,
        domains,
    )
    .expect("the notice must splice into the frame");

    let walked = walk(&records);
    // The notice rides INSIDE the frame, immediately before FRAME_END. A
    // consumer therefore cannot read the published hour without also reading
    // which of its domains are unknown.
    let (notice_sequence, notice_domain, notice_type, notice_payload) =
        walked[walked.len() - 2].clone();
    assert_eq!(notice_domain, compact_contract::COMPACT_DOMAIN_FRAME);
    assert_eq!(
        notice_type,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_DEGRADED
    );
    let (frame_end_sequence, _, frame_end_type, _) = walked[walked.len() - 1];
    assert_eq!(frame_end_type, COMPACT_RECORD_TYPE_FRAME_END);
    // FRAME_END is renumbered past the notice, so the stream sequence stays
    // contiguous -- a gap here is a hard boundary violation downstream.
    assert_eq!(notice_sequence, frame_end_sequence_before);
    assert_eq!(frame_end_sequence, notice_sequence + 1);
    assert_eq!(next_sequence, frame_end_sequence + 1);

    let notice =
        compact_contract::frame_degraded::FrameDegradedRecordV1::decode(&notice_payload).unwrap();
    assert_eq!(notice.degraded_hour, TEST_HOUR);
    assert_eq!(notice.declared_domain_ids(), vec![29, 47]);
    // Every entry carries a typed reason. An entry with an empty reason is
    // refused by the codec, so a published degradation always says why.
    assert!(notice.domains.iter().all(|domain| !domain.reason.is_empty()));
}

/// Regression for the live-only input state the hand-built happy-path fixture
/// above did not exercise: worker-join migration recorded SUCCESS, then the
/// returned reader published a non-clean PERF status from a downstream site
/// that has not yet been migrated into the outcome ledger.
///
/// Production change that makes this fail: treating any member below as fully
/// migrated (removing it from `PARTIALLY_MIGRATED_OUTCOME_DOMAINS`) before its
/// ordinary failure-producing sites record a failure outcome. That recreates
/// the t5 `migrated_site_published_outcome` frame finding and the verdict becomes
/// `Refused` instead of `Degraded`.
#[test]
fn a_worker_success_does_not_turn_a_partially_migrated_domain_failure_into_a_frame_fault() {
    for domain_id in PARTIALLY_MIGRATED_OUTCOME_DOMAINS {
        let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
        record_read_succeeded(
            &mut outcomes,
            *domain_id,
            compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
        );
        let (records, _) =
            assembled_frame(&[(*domain_id, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
        let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);

        assert_eq!(
            findings
                .iter()
                .filter(|finding| finding.frame_finding.is_some())
                .count(),
            0,
            "partially migrated domain {domain_id} recreated a frame-level migration finding: {:?}",
            findings.iter().map(|finding| &finding.text).collect::<Vec<_>>()
        );
        let FrameCompletenessVerdict::Degraded(degraded) =
            classify_frame_completeness(&findings)
        else {
            panic!("partially migrated domain {domain_id} must remain a per-domain degradation");
        };
        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].domain_id, *domain_id);
        assert!(degraded[0]
            .reason
            .contains(compact_contract::diagnostic_channel::PREDICATE_UNDECLARED_NAME));
    }
}

/// Closes live_unlock/t9 and t10. D65 and D66 used to migrate only their
/// worker-join boundary: a normal join recorded `worker_completed` while a
/// failure found inside either reader reached PERF through the legacy
/// status/diagnostic channel alone, so the frame saw a failing record with only
/// a success behind it. Their emit sites now run `record_reader_status_outcome`
/// over whatever bits the read added to the frame's base, so the deciding
/// failure IS in the ledger and the walk classifies it `Declared` — a
/// per-domain degradation carrying a real predicate, not the reserved
/// `undeclared` and not a frame fault.
///
/// Production change that makes this fail: dropping the
/// `record_reader_status_outcome` call from either emit site (the domains are
/// no longer excepted, so the walk reports `migrated_site_published_outcome`
/// and refuses the frame), or putting either domain back on the exception list.
#[test]
fn d65_and_d66_reader_failures_declare_their_own_outcome() {
    for domain_id in [
        COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE,
        COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE,
    ] {
        assert!(
            !PARTIALLY_MIGRATED_OUTCOME_DOMAINS.contains(&domain_id),
            "domain {domain_id} no longer needs the partial-migration exception"
        );
        let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
        record_read_succeeded(
            &mut outcomes,
            domain_id,
            compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
        );
        // Exactly what the emit site does: the frame base was clean and the
        // read came back with SOURCE_UNAVAILABLE | PARTIAL_DATA on it.
        let published = record_reader_status_outcome(
            &mut outcomes,
            domain_id,
            0,
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            || format!("domain {domain_id} walk refused at a guard"),
            Vec::new(),
        );
        assert_eq!(
            published,
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            "the published word must be the derived word"
        );
        let (records, _) = assembled_frame(&[(domain_id, published)]);
        let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);

        assert!(
            findings.iter().all(|finding| finding.frame_finding.is_none()),
            "domain {domain_id} reader failure became a frame fault: {:?}",
            findings
                .iter()
                .map(|finding| &finding.text)
                .collect::<Vec<_>>()
        );
        let FrameCompletenessVerdict::Degraded(degraded) = classify_frame_completeness(&findings)
        else {
            panic!("domain {domain_id} reader failure must degrade only that domain");
        };
        assert_eq!(degraded.len(), 1);
        assert_eq!(degraded[0].domain_id, domain_id);
        assert!(
            !degraded[0]
                .reason
                .contains(compact_contract::diagnostic_channel::PREDICATE_UNDECLARED_NAME),
            "domain {domain_id} still reaches the record undeclared: {}",
            degraded[0].reason
        );
        assert!(
            degraded[0].reason.contains("walk refused at a guard"),
            "domain {domain_id} lost the producer's reason: {}",
            degraded[0].reason
        );
    }
}

/// The behavioural test above proves the classification is right once the emit
/// site records the outcome. This proves the emit sites still do — every domain
/// here is off `PARTIALLY_MIGRATED_OUTCOME_DOMAINS`, so a deleted call is not a
/// lost diagnostic, it is a refused frame.
///
/// D18 joined the list when its CIM1 membership sub-statuses started reaching
/// the published PERF word. Before that, `runs/live_unlock/t21` delivered 282
/// d18 hours whose cores read had failed, whose PERF word was `0x0`, and which
/// the frame's `H4FD` notice therefore never named — while the host ledger
/// recorded 282 source-unavailable domain-hours for the same domain from the
/// same capture.
///
/// D91 joined for the same reason on the other side of the same wire: its emit
/// site published the exact reader's SOURCE_UNAVAILABLE word with nothing in
/// the ledger, so the completeness walk classified it `MigratedButSilent` and
/// raised `migrated_site_published_outcome` as a FRAME finding -- t55 carried
/// that finding on 960 of 960 refused frames.
///
/// Production change that makes this fail: dropping the
/// `record_reader_status_outcome` call from D18's, D65's, D66's or D91's emit
/// block in `build_compact_extraction_gate_records`, pointing one of them at a
/// different domain, or adding a fifth site without deciding whether the domain
/// it speaks for belongs off `PARTIALLY_MIGRATED_OUTCOME_DOMAINS`.
#[test]
fn d18_d65_d66_and_d91_emit_sites_route_their_status_word_through_the_ledger() {
    const SOURCE: &str = include_str!("../attach_log.rs");
    let lines: Vec<&str> = SOURCE.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with("fn build_compact_extraction_gate_records("))
        .expect("the gate function is in this file");
    let end = start
        + 1
        + lines[start + 1..]
            .iter()
            .position(|line| *line == "}")
            .expect("the gate function ends at a column-zero brace");

    let mut routed: Vec<&str> = Vec::new();
    for (offset, line) in lines[start..end].iter().enumerate() {
        if !line
            .trim()
            .starts_with("read.status_flags = record_reader_status_outcome(")
        {
            continue;
        }
        // The ledger argument is first, the domain constant second.
        let domain = lines[start + offset + 1..(start + offset + 5).min(end)]
            .iter()
            .map(|line| line.trim().trim_end_matches(','))
            .find(|line| line.starts_with("COMPACT_DOMAIN_"))
            .expect("the call names the domain it speaks for");
        routed.push(domain);
    }
    routed.sort_unstable();
    assert_eq!(
        routed,
        vec![
            "COMPACT_DOMAIN_AIR_SUPERIORITY_INPUTS",
            "COMPACT_DOMAIN_AIR_VARIANT_RESERVES",
            "COMPACT_DOMAIN_COUNTRY_IDENTITY",
            "COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE",
            "COMPACT_DOMAIN_RAID_INSTANCES_LIVE",
            "COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE",
        ],
        "every emit site that publishes a reader-computed status word must substantiate it; \
         D36 and D38 joined D18/D65/D66/D91 when lane LC closed the silent-PERF class"
    );
}

/// `record_reader_status_outcome` has to be TOTAL over the failure bits it
/// speaks for, because that totality is the whole safety argument for taking
/// D65 and D66 off `PARTIALLY_MIGRATED_OUTCOME_DOMAINS`: no enumeration of the
/// readers' guards is being trusted, only the claim that every bit a reader can
/// add is named here and derives back to the same word.
///
/// Production change that makes this fail: adding a bit to
/// `READER_OWNED_FAILURE_BITS` without teaching the mapping to reproduce it, or
/// letting a non-failing outcome be recorded for a failing word.
#[test]
fn record_reader_status_outcome_is_total_over_the_bits_it_claims() {
    let domain_id = COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE;
    let mut bit = 1u32;
    let mut covered = 0u32;
    while bit != 0 {
        if READER_OWNED_FAILURE_BITS & bit != 0 {
            let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
            let published =
                record_reader_status_outcome(&mut outcomes, domain_id, 0, bit, String::new, Vec::new());
            assert_eq!(published, bit, "bit {bit:#010x} was not derived back");
            let outcome = outcomes
                .get(domain_id)
                .unwrap_or_else(|| panic!("bit {bit:#010x} recorded no outcome"));
            assert!(
                outcome.is_failure(),
                "bit {bit:#010x} recorded a non-failure outcome, which the walk reads as silence"
            );
            assert_eq!(
                outcome.own_bits(),
                bit,
                "bit {bit:#010x} derives to a different word than the reader published"
            );
            covered |= bit;
        }
        bit <<= 1;
    }
    assert_eq!(covered, READER_OWNED_FAILURE_BITS);

    // A clean read records nothing and leaves the join's success standing.
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
    );
    let published =
        record_reader_status_outcome(&mut outcomes, domain_id, 0, 0, String::new, Vec::new());
    assert_eq!(published, 0);
    assert!(!outcomes.get(domain_id).expect("success stands").is_failure());

    // A panicked worker is already diagnosed, and its panic message is the only
    // artefact naming the cause. Deriving a second failure from the stand-in
    // read's bits would displace it with a restatement of those bits.
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_worker_panic_outcome(&mut outcomes, domain_id, &"the walk tripped an assert");
    let panic_status = COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA;
    let published = record_reader_status_outcome(
        &mut outcomes,
        domain_id,
        0,
        panic_status,
        || "a restatement of the bits".to_string(),
        Vec::new(),
    );
    assert_eq!(published, panic_status);
    let outcome = outcomes.get(domain_id).expect("the panic outcome stands");
    assert!(outcome.is_failure());
    assert!(
        outcome.reason.contains("the walk tripped an assert"),
        "the panic message was displaced: {}",
        outcome.reason
    );
    assert!(
        outcome.audit_published_status(panic_status).is_ok(),
        "the panic outcome must still agree with the published word"
    );

    // A frame-scoped bit the derivation CAN reproduce is declared, with the
    // authority pointing at the frame rather than at this domain: the read
    // completed, and the record is an offender anyway.
    for (frame_bit, expected_predicate) in [
        (
            COMPACT_STATUS_NO_CLOCK,
            compact_contract::domain_outcome::READ_PREDICATE_FRAME_HOUR_READABLE,
        ),
        (
            COMPACT_STATUS_CONTROL_INVALID,
            compact_contract::domain_outcome::READ_PREDICATE_READ_AUTHORITY_VALID,
        ),
    ] {
        let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
        let published = record_reader_status_outcome(
            &mut outcomes,
            domain_id,
            frame_bit,
            frame_bit,
            || "the reader did not fail".to_string(),
            Vec::new(),
        );
        assert_eq!(published, frame_bit);
        let outcome = outcomes
            .get(domain_id)
            .unwrap_or_else(|| panic!("frame bit {frame_bit:#010x} recorded no outcome"));
        assert!(outcome.is_failure());
        assert_eq!(outcome.predicate.code.get(), expected_predicate);
        assert_eq!(
            outcome.authority,
            compact_contract::diagnostic_channel::Authority::OtherDomain(
                compact_contract::COMPACT_DOMAIN_FRAME
            ),
            "an inherited frame bit is not this domain's fault"
        );
        assert!(outcome.audit_published_status(frame_bit).is_ok());
        assert!(
            !outcome.reason.contains("the reader did not fail"),
            "the reader's reason must not be attached to a fault it did not have"
        );
    }

    // A frame-scoped bit no `DomainOutcome` field can derive is NOT claimed:
    // an outcome that cannot reproduce it would make the record and the
    // diagnosis disagree. Both readers refuse before this can happen; see
    // `d65_and_d66_refuse_a_profile_mismatch_with_a_bit_of_their_own`.
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    let published = record_reader_status_outcome(
        &mut outcomes,
        domain_id,
        COMPACT_STATUS_PROFILE_MISMATCH,
        COMPACT_STATUS_PROFILE_MISMATCH,
        String::new,
        Vec::new(),
    );
    assert_eq!(published, COMPACT_STATUS_PROFILE_MISMATCH);
    assert!(outcomes.get(domain_id).is_none());
}

/// The safety argument for taking D65 and D66 off
/// `PARTIALLY_MIGRATED_OUTCOME_DOMAINS` has to cover the FRAME's bits too: a
/// domain whose read succeeded still publishes a failing word if the frame's
/// prologue put a failure bit in the base, and with no outcome behind it that
/// is the `MigratedButSilent` finding that refuses the hour.
///
/// This walks all five declared frame-scoped writes and pins how each is
/// covered. `PROFILE_MISMATCH` is the one that needs a producer change: both
/// readers now refuse it rather than walk offsets measured against a different
/// build, which gives the emit site a bit of the domain's OWN to substantiate.
///
/// Production change that makes this fail: removing either reader's
/// profile-mismatch refusal, or adding a sixth frame-scoped base status write
/// whose bit neither the derivation nor a reader accounts for.
#[test]
fn d65_and_d66_refuse_a_profile_mismatch_with_a_bit_of_their_own() {
    let state = super::tests::empty_collector_state();
    let d66 = read_compact_occupation_state_extras(&state, COMPACT_STATUS_PROFILE_MISMATCH);
    assert!(d66.rows.is_empty());
    assert_ne!(d66.status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert_eq!(
        d66.first_failure.map(|failure| failure.kind),
        Some(CompactOccupationExtrasFailureKind::ProfileMismatch)
    );

    let d65 = read_compact_special_project_programs(&state, COMPACT_STATUS_PROFILE_MISMATCH);
    assert!(d65.rows.is_empty());
    assert_ne!(d65.status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert_eq!(
        d65.first_failure.map(|failure| failure.reason),
        Some(CompactSpecialProjectProgramFailureReason::ProfileMismatch)
    );

    // And the bit they added is one the emit site can substantiate, so the
    // record reaches the completeness walk Declared rather than silent.
    for (domain_id, status) in [
        (COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE, d66.status_flags),
        (COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE, d65.status_flags),
    ] {
        let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
        record_read_succeeded(
            &mut outcomes,
            domain_id,
            compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
        );
        let published = record_reader_status_outcome(
            &mut outcomes,
            domain_id,
            COMPACT_STATUS_PROFILE_MISMATCH,
            status,
            || "the walk refused an unverified build profile".to_string(),
            Vec::new(),
        );
        assert_eq!(published, status);
        let (records, _) = assembled_frame(&[(domain_id, published)]);
        let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);
        assert!(
            findings.iter().all(|finding| finding.frame_finding.is_none()),
            "domain {domain_id} under a profile mismatch became a frame fault: {:?}",
            findings
                .iter()
                .map(|finding| &finding.text)
                .collect::<Vec<_>>()
        );
    }
}

/// The complementary guard: this patch must not globally downgrade a bypassed
/// migrated site. D72's ordinary failure sites are wired into the ledger, so a
/// failing PERF word accompanied only by a success outcome is still an internal
/// producer contradiction and must refuse the frame.
///
/// Production change that makes this fail: adding D72 to the partial-migration
/// exception, or removing the `MigratedButSilent` frame finding from the
/// completeness walk.
#[test]
fn a_fully_migrated_site_that_publishes_failure_after_only_success_still_refuses() {
    let domain_id = COMPACT_DOMAIN_RESOLVED_TOOLTIP_INSTANCES_LIVE;
    assert!(!PARTIALLY_MIGRATED_OUTCOME_DOMAINS.contains(&domain_id));
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_PAYLOAD_ENCODED,
    );
    let (records, _) = assembled_frame(&[(domain_id, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);

    assert!(findings.iter().any(|finding| {
        finding.frame_finding.as_ref().is_some_and(|frame| {
            frame.predicate.name
                == compact_contract::domain_outcome::READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME_NAME
        })
    }));
    assert!(matches!(
        classify_frame_completeness(&findings),
        FrameCompletenessVerdict::Refused
    ));
}

/// A domain inside its expected-partial tolerance is NOT a degradation. The
/// frame stays complete and no notice is produced -- the two states must never
/// collapse into one another.
#[test]
fn a_tolerated_partial_leaves_the_frame_complete_and_produces_no_notice() {
    for tolerance in compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES {
        let (records, _) = assembled_frame(&[(18, 0), (tolerance.domain_id, tolerance.tolerated_flags)]);
        let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &no_outcomes());
        assert!(
            findings.is_empty(),
            "domain {} inside its tolerance must not object",
            tolerance.domain_id
        );
        assert!(matches!(
            classify_frame_completeness(&findings),
            FrameCompletenessVerdict::Complete
        ));
    }
}

/// A fault in the FRAME still refuses the whole frame. A degraded frame is only
/// meaningful if a consumer can attribute the degradation to a specific,
/// well-formed hour, so anything that undermines the frame's identity, boundary
/// or grammar keeps the old all-or-nothing verdict.
#[test]
fn a_frame_level_fault_still_refuses_the_whole_frame() {
    // 1. A record stamped with a different hour than the frame: the frame's
    //    identity is not trustworthy.
    let (records, _) = assembled_frame(&[(18, 0)]);
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR + 1, &no_outcomes());
    assert!(!findings.is_empty());
    assert!(
        matches!(
            classify_frame_completeness(&findings),
            FrameCompletenessVerdict::Refused
        ),
        "an hour mismatch must refuse, never degrade"
    );

    // 2. No FRAME_END: the frame has no boundary.
    let mut headless = Vec::new();
    append_compact_record(
        &mut headless,
        1,
        TEST_HOUR,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload_with_status(0),
    );
    let findings = compact_frame_completeness_findings(&headless, TEST_HOUR, &no_outcomes());
    assert!(matches!(
        classify_frame_completeness(&findings),
        FrameCompletenessVerdict::Refused
    ));

    // 3. A FRAME_END carrying a failure status: the frame's own telemetry says
    //    the hour is bad, which is not one domain's problem.
    let mut bad_frame_end = Vec::new();
    let mut frame_end = frame_end_payload();
    frame_end[24..28].copy_from_slice(&COMPACT_STATUS_NO_CLOCK.to_le_bytes());
    append_compact_record(
        &mut bad_frame_end,
        1,
        TEST_HOUR,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload_with_status(0),
    );
    append_compact_record(
        &mut bad_frame_end,
        2,
        TEST_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end,
    );
    let findings = compact_frame_completeness_findings(&bad_frame_end, TEST_HOUR, &no_outcomes());
    assert!(matches!(
        classify_frame_completeness(&findings),
        FrameCompletenessVerdict::Refused
    ));
}

/// The case that matters most, and the one the other refusal test misses: a
/// framing fault ALONGSIDE per-domain faults.
///
/// A mutation that made framing findings fall through instead of refusing was
/// invisible to the test above, because with only framing findings the degraded
/// list came out empty and the fallback still refused. That is precisely the
/// dangerous shape -- a frame with an untrustworthy hour AND a failing domain
/// would have published, with the notice quietly vouching for the rest of a
/// frame whose identity was never established.
#[test]
fn a_framing_fault_refuses_even_when_per_domain_faults_could_have_degraded() {
    let (records, _) = assembled_frame(&[(18, 0), (29, COMPACT_STATUS_PARTIAL_DATA)]);
    let outcomes = outcomes_for(&[(29, COMPACT_STATUS_PARTIAL_DATA)]);
    // Walking against the wrong hour makes EVERY record raise a framing
    // finding, while domain 29 also raises its per-domain objection. The
    // per-domain half must not be able to carry the frame.
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR + 1, &outcomes);
    assert!(
        findings.iter().any(|finding| finding.offender.is_none()),
        "the mismatched hour must raise a framing finding"
    );
    assert!(
        findings.iter().any(|finding| finding.offender.is_some()),
        "domain 29 must still raise its own objection"
    );
    assert!(
        matches!(
            classify_frame_completeness(&findings),
            FrameCompletenessVerdict::Refused
        ),
        "one framing fault must veto publication no matter how many domains \
         could otherwise have been degraded"
    );
}

/// A domain whose status word could not even be READ cannot be classified, so
/// it must not be published as degraded. Publishing it would assert bits nobody
/// read -- the mislabel this record exists to prevent, in its worst form.
#[test]
fn a_perf_record_too_short_to_carry_a_status_refuses_rather_than_guessing() {
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        TEST_HOUR,
        29,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &[0u8; 8],
    );
    append_compact_record(
        &mut records,
        2,
        TEST_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(),
    );
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &no_outcomes());
    assert!(!findings.is_empty());
    assert!(
        matches!(
            classify_frame_completeness(&findings),
            FrameCompletenessVerdict::Refused
        ),
        "an unreadable status must refuse, not be guessed at"
    );
}

/// A failing domain that says nothing still gets a non-empty, countable reason.
///
/// Some scheduled domains attach no diagnostic units at all. Their failures
/// used to arrive as a bare status bit, and an entry with an EMPTY reason is
/// refused by the codec -- so without a name for the condition, a silent domain
/// would have made its whole frame unpublishable. That is still the property,
/// and it is still what this test is for.
///
/// REWRITTEN twice over by the producer migration, in both its subject and its
/// literal, and neither change weakens it:
///
/// * SUBJECT. Domain 29 is in `MIGRATED_OUTCOME_DOMAINS`, and a migrated site
///   records an outcome on its failure path by construction -- so a failing
///   migrated domain that says nothing is a producer regression, which the walk
///   now reports as a frame-level finding and which REFUSES the frame. That is
///   correct, and it means "a silent domain degrades" is no longer a thing that
///   can be asked of a migrated domain at all. It is still exactly the question
///   for a domain the migration has not reached, so the test asks it there, and
///   asserts the domain's non-migrated status so that migrating it later turns
///   this red rather than silently retargeting the test.
/// * LITERAL. The shared name is no longer the string
///   `FRAME_DEGRADED_REASON_UNDIAGNOSED` composed by the collector; it is the
///   reserved `Predicate::undeclared()` the offender carries, rendered into the
///   reason. The property -- one SHARED, non-empty, countable marker, the same
///   for every silent domain -- is asserted directly instead of through the
///   literal, which is what it was standing in for.
#[test]
fn a_failing_domain_with_no_diagnostic_units_is_named_undiagnosed_not_blank() {
    // Not migrated, and asserted rather than assumed: the whole scenario is
    // "the producer declared nothing", which for a migrated domain is a frame
    // fault instead.
    const SILENT_DOMAIN: u16 = 19;
    const OTHER_SILENT_DOMAIN: u16 = 25;
    for domain_id in [SILENT_DOMAIN, OTHER_SILENT_DOMAIN] {
        assert!(
            !MIGRATED_OUTCOME_DOMAINS.contains(&domain_id),
            "domain {domain_id} has been migrated; this test needs a domain whose producer              genuinely declares nothing, so pick another one -- do not delete the assertion"
        );
        assert!(
            compact_contract::expected_partial_tolerance(domain_id).is_none(),
            "domain {domain_id} now tolerates partials; its failure would not degrade"
        );
    }

    let (records, _) = assembled_frame(&[(SILENT_DOMAIN, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &no_outcomes());
    let FrameCompletenessVerdict::Degraded(domains) = classify_frame_completeness(&findings) else {
        panic!("a lone per-domain fault must degrade");
    };
    // Non-empty: an empty reason is what the codec refuses, and refusing it
    // here is what would have re-deadlocked the frame.
    assert!(
        !domains[0].reason.is_empty(),
        "a silent domain must still carry a reason"
    );
    // SHARED: a second, different silent domain must produce the SAME marker,
    // because that is what makes the silence countable in one pass over a
    // stream rather than a per-domain string a consumer has to enumerate.
    let (other_records, _) =
        assembled_frame(&[(OTHER_SILENT_DOMAIN, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    let other_findings =
        compact_frame_completeness_findings(&other_records, TEST_HOUR, &no_outcomes());
    let FrameCompletenessVerdict::Degraded(other_domains) =
        classify_frame_completeness(&other_findings)
    else {
        panic!("a lone per-domain fault must degrade");
    };
    // Two places carry the marker, and both are asserted because they are read
    // by different consumers. The OFFENDER carries the reserved predicate, which
    // is the machine-countable form; the DEGRADED entry carries the reason text,
    // which is what an existing consumer logs.
    let undeclared = compact_contract::diagnostic_channel::Predicate::undeclared();
    for (label, found) in [("first", &findings), ("second", &other_findings)] {
        let offender = found
            .iter()
            .find_map(|finding| finding.offender.as_ref())
            .unwrap_or_else(|| panic!("the {label} silent domain must produce an offender"));
        assert_eq!(
            offender.predicate, undeclared,
            "a producer that declared nothing must be stamped with the reserved              undeclared predicate, which is what makes the silence countable"
        );
    }
    // SHARED: strip the per-domain prefix and the two reasons must be the SAME
    // string. Asserting equality rather than a literal is deliberate -- it is
    // the property ("one name for every silent domain") rather than today's
    // spelling of it, so rewording the marker keeps this green while giving any
    // ONE domain its own variant turns it red.
    let strip = |domain_id: u16, reason: &str| {
        reason
            .strip_prefix(&format!("domain {domain_id}: "))
            .unwrap_or(reason)
            .to_string()
    };
    assert_eq!(
        strip(SILENT_DOMAIN, &domains[0].reason),
        strip(OTHER_SILENT_DOMAIN, &other_domains[0].reason),
        "every silent domain must be named by the SAME marker, not a per-domain one"
    );
    // And it is publishable: a silent domain must not be able to re-deadlock a
    // whole frame just by failing to explain itself.
    let mut records = records;
    let mut next_sequence = 3u64;
    insert_frame_degraded_notice_before_frame_end(
        &mut records,
        &mut next_sequence,
        TEST_HOUR,
        domains,
    )
    .expect("an undiagnosed failure must still be statable");
}

/// The correctness guard for cross-frame deltas, proved for EVERY domain rather
/// than argued for one.
///
/// A degraded domain publishes nothing usable this hour, and the recorder fold
/// discards its record. If the collector advanced that domain's delta baseline
/// anyway, the rows the discarded frame carried would be marked delivered and
/// never re-sent, and the fold accumulator would diverge for the rest of the
/// campaign. That is why the degraded publication path can commit the frame's
/// trackers wholesale: the freeze is per domain and already happened at
/// row-build time.
///
/// Degraded implies withheld, universally: `Degraded` means the NORMALIZED
/// status has failure-class bits, normalizing only ever clears bits, so the raw
/// status has them too -- which is exactly `compact_delta_baseline_withheld`.
#[test]
fn every_degraded_status_withholds_its_delta_baseline_for_every_domain() {
    let tolerance_domains: Vec<u16> = compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES
        .iter()
        .map(|tolerance| tolerance.domain_id)
        .collect();
    // Domains with lane-local status bits (49/51/53/60/99) are included on
    // purpose: their badges are stripped before classification, and this is
    // where a normalization mistake would show up as a degraded domain whose
    // baseline was allowed to advance.
    let mut domains: Vec<u16> = vec![18, 29, 30, 47, 49, 51, 53, 60, 91, 99];
    domains.extend(tolerance_domains);

    let mut degraded_seen = 0usize;
    for domain_id in domains {
        for first in 0..32u32 {
            for second in 0..32u32 {
                let status = (1u32 << first) | (1u32 << second);
                if compact_contract::frame_degraded::compact_domain_completeness(domain_id, status)
                    .is_degraded()
                {
                    degraded_seen += 1;
                    assert!(
                        compact_delta_baseline_withheld(status),
                        "domain {domain_id} status {status:#010x} is degraded but its delta \
                         baseline would advance"
                    );
                }
            }
        }
    }
    assert!(
        degraded_seen > 0,
        "the sweep must actually reach the degraded classification"
    );
}

/// Where the degraded notice STOPS, pinned so it is inherited as a fact rather
/// than rediscovered.
///
/// The completeness gate builds per-domain objections only inside its
/// `DOMAIN_PERF` branch. It has no `DOMAIN_GAP` branch at all, so a domain that
/// arrives as a gap is structurally unable to become a named degraded domain --
/// and `replace_rejected_domain_records_with_gaps` (this file) deletes the
/// emitted records of any domain outside the clock brackets and substitutes a
/// `ClockUnreadable` gap BEFORE this walk ever runs.
///
/// That is not a silent pass-through on the wire: `DOMAIN_GAP` is a first-class
/// typed outcome and the recorder fold already treats a gapped domain as unknown
/// for the hour, so such a domain is never mistaken for complete. What IS wrong,
/// and is a pre-existing defect this record does not fix, is that the
/// substituted reason says `ClockUnreadable` whatever the real cause was. A
/// uniform per-domain outcome contract has to reach that substitution point, or
/// it will keep laundering causes exactly here.
#[test]
fn a_gapped_domain_is_accounted_by_its_gap_record_and_never_by_the_notice() {
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        TEST_HOUR,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload_with_status(0),
    );
    // A gap for domain 29. Its contents are irrelevant to this assertion and
    // that is the finding: the gate never inspects a gap record at all.
    append_compact_record(
        &mut records,
        2,
        TEST_HOUR,
        29,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &[0u8; compact_contract::COMPACT_DOMAIN_GAP_PAYLOAD_LEN],
    );
    append_compact_record(
        &mut records,
        3,
        TEST_HOUR,
        compact_contract::COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(),
    );
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &no_outcomes());
    assert!(
        findings.is_empty(),
        "a gapped domain raises no completeness finding; its H4DG record is its account"
    );
    assert!(
        matches!(
            classify_frame_completeness(&findings),
            FrameCompletenessVerdict::Complete
        ),
        "a frame whose only absent domain is gapped is complete as far as this gate \
         is concerned -- the gap is the accounting, and this is the exact edge a \
         uniform per-domain outcome contract must extend to"
    );
}

/// Two objections for one domain mean the frame's per-domain PERF cardinality
/// is broken. That is a frame fault, not a degradation, and the notice must not
/// silently merge them -- a merged entry would pick one domain's bits and
/// discard the other's.
#[test]
fn one_domain_objecting_twice_is_a_frame_fault_not_a_merged_entry() {
    let (records, _) =
        assembled_frame(&[(29, COMPACT_STATUS_PARTIAL_DATA), (29, COMPACT_STATUS_NO_CLOCK)]);
    // The ledger holds ONE outcome per domain, which is the production shape:
    // a domain reads once and diagnoses once. That is exactly why this frame is
    // broken, and it is why the count below is no longer pinned -- see the
    // assertion's own comment.
    let findings = compact_frame_completeness_findings(
        &records,
        TEST_HOUR,
        &outcomes_for(&[(29, COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_NO_CLOCK)]),
    );
    // The exact COUNT is deliberately not asserted any more. It used to be 2 --
    // one objection per duplicated PERF record -- and the producer migration
    // legitimately adds more: each duplicate also publishes a status word that
    // disagrees with the domain's single outcome, which `audit_published_status`
    // reports as its own frame-level finding. Pinning the count would make this
    // test fail whenever the walk gains a NEW true thing to say about the same
    // broken frame, which is not what it is for.
    //
    // What it is for is asserted below, and neither half is weakened: both
    // duplicated objections are still raised as per-domain offenders, and the
    // verdict is still REFUSED rather than a merged degradation.
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding
                .offender
                .as_ref()
                .is_some_and(|offender| offender.domain_id == 29))
            .count(),
        2,
        "both of domain 29's objections must survive as offenders, unmerged; findings: {:?}",
        findings.iter().map(|f| f.text.as_str()).collect::<Vec<_>>()
    );
    assert!(matches!(
        classify_frame_completeness(&findings),
        FrameCompletenessVerdict::Refused
    ));
}

/// Splicing into a frame that is not shaped the way the scheduler builds them
/// fails closed. The caller turns that into a refusal, so a degradation that
/// cannot be STATED never becomes a silently-degraded frame.
#[test]
fn a_notice_that_cannot_be_spliced_fails_closed() {
    let degraded = vec![compact_contract::frame_degraded::FrameDegradedDomain::from_status(
        29,
        COMPACT_STATUS_PARTIAL_DATA,
        "d29_invalid_air_wing_rows=3",
    )
    .unwrap()];

    // No FRAME_END at all.
    let mut headless = Vec::new();
    append_compact_record(
        &mut headless,
        1,
        TEST_HOUR,
        29,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload_with_status(COMPACT_STATUS_PARTIAL_DATA),
    );
    let mut next_sequence = 2u64;
    assert!(insert_frame_degraded_notice_before_frame_end(
        &mut headless,
        &mut next_sequence,
        TEST_HOUR,
        degraded.clone(),
    )
    .is_err());

    // FRAME_END for a different hour than the notice.
    let (mut records, mut next_sequence) = assembled_frame(&[(29, COMPACT_STATUS_PARTIAL_DATA)]);
    assert!(insert_frame_degraded_notice_before_frame_end(
        &mut records,
        &mut next_sequence,
        TEST_HOUR + 1,
        degraded,
    )
    .is_err());
}

/// The degraded summary the capture loop writes to `collector_worker_error.log`
/// must reach the operator WHOLE.
///
/// This is the artifact a human reads when a capture publishes an hour with
/// domains marked unknown, and it carries one typed reason per degraded domain.
/// A single worker-error record bounds its text at `RECORD_TEXT_LIMIT`; the
/// summary for a fully degraded roster is an order of magnitude past that, so
/// the record used to end mid-domain and every reason after the cut was
/// destroyed -- exactly when the run most needed diagnosing.
///
/// Production change that makes this fail: `attach_worker_error_record_bytes`
/// calling the single-record `format_record` instead of `format_records`.
#[test]
fn a_full_roster_degraded_summary_reaches_the_log_whole() {
    use compact_contract::worker_error_record as record_format;

    // The shape the capture loop builds, at the scheduled roster's size, with
    // each reason at the wire's own per-domain bound.
    let reason_len = compact_contract::frame_degraded::FRAME_DEGRADED_MAX_REASON_LEN;
    let last_domain = 99u16;
    let summary = (0..73u16)
        .map(|index| {
            let domain = if index == 72 { last_domain } else { index };
            // ASCII only, so the text appears verbatim in a `{:?}` rendering
            // and this test is not measuring the escape rules.
            format!(
                "d{domain}:{:#010x}[{}]",
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
                "Z".repeat(reason_len)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!("hour {TEST_HOUR} published with degraded domains: {summary}");

    let records = attach_worker_error_record_bytes("frame_degraded", &text);
    assert!(
        records.len() > 1,
        "a full-roster summary must not be squeezed into one record"
    );

    // Every record is independently intact and attributable to this process,
    // which is what lets the host read them at all.
    let pid = unsafe { GetCurrentProcessId() };
    let creation = current_process_creation_filetime().unwrap_or(0);
    let header = record_format::owned_header(pid, creation);
    let suffix = record_format::owned_suffix(pid);
    let mut carried = String::new();
    for record in &records {
        let line = record.strip_suffix('\n').expect("record ends with a newline");
        let body = record_format::owned_body(line.as_bytes(), &header, &suffix)
            .expect("every chunk must be an intact record the host can own");
        carried.push_str(&String::from_utf8_lossy(body));
    }

    // The LAST degraded domain's reason is the one a truncating writer loses.
    let tail = format!("d{last_domain}:{:#010x}[", COMPACT_STATUS_SOURCE_UNAVAILABLE);
    assert!(
        carried.contains(&tail),
        "the last degraded domain's reason must survive to the log"
    );
    assert!(
        !record_format::format_record(pid, creation, "frame_degraded", &text).contains(&tail),
        "the fixture must be large enough that a single record WOULD have lost the tail; \
         otherwise this test cannot fail"
    );
}

// ---------------------------------------------------------------------------
// The silent-PERF class, closed for every declared domain at once
// ---------------------------------------------------------------------------

/// Every failure bit a DOMAIN READ can put on its own PERF word — which is
/// exactly `READER_OWNED_FAILURE_BITS`, spelled out here so that adding a bit to
/// that constant without extending this list is visible.
///
/// The other four members of `COMPACT_STATUS_FAILURE_MASK` are absent on
/// purpose, and `a_frame_prologue_bit_alone_is_left_to_refuse_the_frame` below
/// is where that is asserted rather than assumed.
const EVERY_READER_OWNED_FAILURE_BIT: &[u32] = &[
    COMPACT_STATUS_SOURCE_UNAVAILABLE,
    COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_WORKER_PANIC,
    COMPACT_STATUS_NO_CLOCK,
    COMPACT_STATUS_CONTROL_INVALID,
];

/// THE CLASS KILLER.
///
/// `migrated_site_published_outcome` was fixed for D91 by lane KT and then
/// reported again, by the very next capture, for domains 36 and 38. It is not a
/// per-domain defect: `MIGRATED_OUTCOME_DOMAINS` declares a domain migrated on
/// the strength of its WORKER JOIN, and every one of those domains whose reader
/// can return a failing status word without panicking is latently in the class.
/// So this asserts the property for EVERY declared domain and EVERY failure bit
/// the contract has, rather than for the two the last capture happened to name.
///
/// The `before` half is what stops this being vacuous: it proves each case
/// really did produce the frame-refusing finding, so a sweep that silently
/// stopped firing would fail here rather than pass trivially.
///
/// Production change that makes this fail: deleting the
/// `substantiate_silent_domain_outcomes` call before `FRAME_END`, narrowing it
/// so it stops covering a bit or a domain, or adding a domain to
/// `MIGRATED_OUTCOME_DOMAINS` whose emit path can bypass it.
#[test]
fn no_migrated_domain_can_publish_a_failing_perf_word_without_an_outcome() {
    let mut exercised = 0usize;
    for domain_id in MIGRATED_OUTCOME_DOMAINS.iter().copied() {
        // The escape hatch is a DIFFERENT state: those domains keep the
        // reserved `undeclared` predicate and never produced the frame finding,
        // so they are not in this class. Their test is
        // `a_worker_success_does_not_turn_a_partially_migrated_domain_failure_into_a_frame_fault`.
        if PARTIALLY_MIGRATED_OUTCOME_DOMAINS.contains(&domain_id) {
            continue;
        }
        for bit in EVERY_READER_OWNED_FAILURE_BIT.iter().copied() {
            let failed =
                compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
                    domain_id, bit,
                );
            if failed == 0 {
                // A tolerated PARTIAL for this domain is not an offence at all,
                // so there is no finding to kill.
                continue;
            }
            // The exact input state the defect needs: the worker join recorded
            // SUCCESS, and nothing else spoke.
            let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
            record_read_succeeded(
                &mut outcomes,
                domain_id,
                compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
            );
            let (records, _) = assembled_frame(&[(domain_id, bit)]);

            // BEFORE: this is the frame-refusing finding, on this domain, now.
            let before = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);
            assert!(
                before.iter().any(|finding| {
                    finding.frame_finding.as_ref().is_some_and(|frame| {
                        frame.predicate.name
                            == compact_contract::domain_outcome::READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME_NAME
                    })
                }),
                "domain {domain_id} with status {bit:#010x} did not reproduce the defect, so \
                 the `after` half below proves nothing"
            );
            assert!(
                matches!(
                    classify_frame_completeness(&before),
                    FrameCompletenessVerdict::Refused
                ),
                "domain {domain_id} with status {bit:#010x} must refuse the frame before the sweep"
            );

            // AFTER: the sweep runs where production runs it.
            let substantiated = substantiate_silent_domain_outcomes(&records, &mut outcomes, 0);
            assert_eq!(
                substantiated, 1,
                "the sweep must substantiate domain {domain_id}'s {bit:#010x} word"
            );
            assert_eq!(
                classify_production_outcome_coverage(domain_id, &outcomes),
                compact_contract::domain_outcome::OutcomeCoverage::Declared,
                "domain {domain_id} with status {bit:#010x} is still silent after the sweep"
            );

            let after = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);
            assert!(
                after.iter().all(|finding| finding.frame_finding.is_none()),
                "domain {domain_id} with status {bit:#010x} still carries a FRAME finding: {:?}",
                after.iter().map(|finding| &finding.text).collect::<Vec<_>>()
            );
            // Substantiating must not trade one frame-refusing finding for
            // another: an outcome claiming a bit the record does not carry
            // would raise `published_status_matches_outcome` instead.
            assert!(
                !after.iter().any(|finding| finding
                    .frame_finding
                    .as_ref()
                    .is_some_and(|frame| frame.predicate.name
                        == "published_status_matches_outcome")),
                "domain {domain_id} with status {bit:#010x} made the record and the diagnosis \
                 disagree: {:?}",
                after.iter().map(|finding| &finding.text).collect::<Vec<_>>()
            );
            // The domain is EXCLUDED AND ACCOUNTED -- the partial-data law --
            // rather than taking the frame down with it.
            let FrameCompletenessVerdict::Degraded(degraded) = classify_frame_completeness(&after)
            else {
                panic!(
                    "domain {domain_id} with status {bit:#010x} must become a per-domain \
                     degradation, not a verdict about the frame"
                );
            };
            assert_eq!(degraded.len(), 1);
            assert_eq!(degraded[0].domain_id, domain_id);
            // ...and it says why, in a string a stream census can count.
            assert!(
                degraded[0].reason.contains("recorded no outcome of its own"),
                "domain {domain_id} with status {bit:#010x} degraded without naming the silence: {}",
                degraded[0].reason
            );
            assert!(
                !degraded[0]
                    .reason
                    .contains(compact_contract::diagnostic_channel::PREDICATE_UNDECLARED_NAME),
                "domain {domain_id} must not fall back to the reserved undeclared predicate"
            );
            exercised += 1;
        }
    }
    // The loop above must actually have run. A `MIGRATED_OUTCOME_DOMAINS` that
    // emptied itself, or a tolerance that swallowed every bit, would otherwise
    // make every assertion above unreachable and this test green.
    assert!(
        exercised >= MIGRATED_OUTCOME_DOMAINS.len(),
        "only {exercised} (domain, bit) cases ran across {} declared domains",
        MIGRATED_OUTCOME_DOMAINS.len()
    );
}

/// The sweep speaks only for the record in front of it.
///
/// A domain that DID diagnose its own failure keeps its producer's reason: the
/// sweep would otherwise overwrite the one artefact naming the real cause with
/// a restatement of the bits, which is the same loss
/// `record_worker_panic_outcome` exists to prevent.
///
/// Production change that makes this fail: dropping the `MigratedButSilent`
/// guard and substantiating every failing record unconditionally.
#[test]
fn the_sweep_never_displaces_a_producers_own_diagnosis() {
    let domain_id = COMPACT_DOMAIN_AIR_VARIANT_RESERVES;
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
    );
    let published = record_reader_status_outcome(
        &mut outcomes,
        domain_id,
        0,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        || "d36 air-variant reserves: the producer's own words".to_string(),
        Vec::new(),
    );
    let (records, _) = assembled_frame(&[(domain_id, published)]);
    assert_eq!(
        substantiate_silent_domain_outcomes(&records, &mut outcomes, 0),
        0,
        "a domain that already diagnosed itself must not be swept"
    );
    assert_eq!(
        compact_contract::domain_outcome::failure_diagnosis(&outcomes, domain_id)
            .expect("the producer's failure is the diagnosis")
            .reason,
        "d36 air-variant reserves: the producer's own words"
    );
}

/// A CLEAN record is not swept. The sweep exists to stop a failing word from
/// refusing the frame; inventing a failure for a domain that succeeded would be
/// the same accounting defect pointed the other way.
///
/// Production change that makes this fail: sweeping on record type alone instead
/// of on the tolerance-adjusted failure flags.
#[test]
fn the_sweep_leaves_a_clean_record_alone() {
    let domain_id = COMPACT_DOMAIN_AIR_SUPERIORITY_INPUTS;
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
    );
    let (records, _) = assembled_frame(&[(domain_id, 0)]);
    assert_eq!(
        substantiate_silent_domain_outcomes(&records, &mut outcomes, 0),
        0
    );
    assert!(
        compact_contract::domain_outcome::failure_diagnosis(&outcomes, domain_id).is_none(),
        "a clean domain must not acquire a failure it never had"
    );
}

/// The BOUNDARY of the sweep, asserted rather than assumed.
///
/// A domain whose only failure bit is one the outcome vocabulary cannot speak
/// for is deliberately NOT swept: `DomainOutcome::is_failure` is decided by
/// completion/extent/clock/authority, and every one of those derives a bit
/// inside `DERIVED_STATUS_MASK`, so any outcome recorded for such a word would
/// claim a bit the record does not carry and trade one frame-refusing finding
/// for `published_status_matches_outcome`.
///
/// That is safe because the state is not reachable from a READ for the domains
/// the sweep covers. `state_unavailable`, `profile_mismatch` and
/// `no_scalar_source` are each set exactly once, in the frame-scoped status
/// prologue, and `bridge_unavailable`'s only `|=` in the tree belongs to a
/// domain on `PARTIALLY_MIGRATED_OUTCOME_DOMAINS`. So reaching here means the
/// FRAME failed, every domain inherited its word, and the frame is refused by
/// the FRAME_END status finding -- the correct verdict for a frame-wide fault,
/// and not the accounting defect the sweep removes.
///
/// Production change that makes this fail: sweeping unconditionally on
/// `failed != 0` (the outcome claims `source_unavailable`, the record does not
/// carry it, and the walk objects), or dropping the FRAME_END status finding
/// that makes refusing the frame here correct for its own reason.
#[test]
fn a_frame_prologue_bit_alone_is_left_to_refuse_the_frame() {
    let domain_id = COMPACT_DOMAIN_AIR_VARIANT_RESERVES;
    for bit in [
        COMPACT_STATUS_PROFILE_MISMATCH,
        COMPACT_STATUS_NO_SCALAR_SOURCE,
        compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
    ] {
        assert_eq!(
            bit & READER_OWNED_FAILURE_BITS,
            0,
            "{bit:#010x} is a reader-owned bit and belongs in the swept set"
        );
        let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
        record_read_succeeded(
            &mut outcomes,
            domain_id,
            compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
        );
        let (records, _) = assembled_frame(&[(domain_id, bit)]);
        assert_eq!(
            substantiate_silent_domain_outcomes(&records, &mut outcomes, bit),
            0,
            "a word an outcome cannot speak for must not be swept"
        );
        let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &outcomes);
        assert!(
            findings.iter().any(|finding| {
                finding.frame_finding.as_ref().is_some_and(|frame| {
                    frame.predicate.name
                        == compact_contract::domain_outcome::READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME_NAME
                })
            }),
            "the finding must remain, loudly, for a word nothing can diagnose"
        );
    }
}

/// The frame's word makes every domain an offender, and the sweep must not turn
/// that into a per-domain accusation against reads that succeeded.
///
/// Production change that makes this fail: dropping the `frame_base_status`
/// argument, or computing `own_bits` against zero -- either makes the outcome
/// say this domain's reader failed to return its rows when the only thing wrong
/// with the hour was the frame's clock.
#[test]
fn a_bit_the_frame_contributed_is_attributed_to_the_frame() {
    let domain_id = COMPACT_DOMAIN_AIR_SUPERIORITY_INPUTS;
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
    );
    let (records, _) = assembled_frame(&[(domain_id, COMPACT_STATUS_NO_CLOCK)]);
    assert_eq!(
        substantiate_silent_domain_outcomes(&records, &mut outcomes, COMPACT_STATUS_NO_CLOCK),
        1
    );
    let outcome = compact_contract::domain_outcome::failure_diagnosis(&outcomes, domain_id)
        .expect("the frame's bit still makes this domain an offender that must be diagnosed");
    assert_eq!(
        outcome.authority,
        compact_contract::diagnostic_channel::Authority::OtherDomain(
            compact_contract::COMPACT_DOMAIN_FRAME
        ),
        "a bit the domain did not add is not the domain's defect"
    );
    // The predicate says what this outcome IS -- a diagnosis reconstructed from
    // the record -- for a frame-contributed bit exactly as for a reader's own.
    // WHO the failure belongs to is the authority above, and that is the field
    // a consumer must read to tell the two apart; overloading the predicate
    // with it would cost the countable silent-producer signal.
    assert_eq!(
        outcome.predicate.code.get(),
        compact_contract::domain_outcome::READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD
    );
    assert!(
        outcome.reason.contains("no_clock"),
        "the reason must still name the bit that offended: {}",
        outcome.reason
    );
}

/// The class killer above proves the SWEEP closes the class. This proves the
/// sweep is actually wired in, which the behavioural test cannot: it calls the
/// helper itself, so deleting the production call leaves it green.
///
/// The rule is positional AND structural: every frame this function seals must
/// have been swept first, from the same block.
/// `build_compact_extraction_gate_records` has two exits that seal a frame --
/// the state-unavailable early return and the ordinary one -- and each
/// `append_compact_frame_end_record(` in it must be preceded by a
/// `substantiate_silent_domain_outcomes(` at the SAME brace depth, with no other
/// frame seal in between.
///
/// The depth is what stops the obvious way to satisfy a textual scan while
/// changing nothing: `if false { substantiate_silent_domain_outcomes(..) }`
/// immediately before the seal reads as preceding it and never runs.
///
/// And it is honest about its limit. A source scan cannot prove the call
/// EXECUTES, only that it sits where execution reaches it unconditionally.
/// Proving execution means driving the gate function, which needs a live game
/// process; the behavioural tests above prove what the sweep DOES, and this
/// proves it is where the frame passes.
///
/// Production change that makes this fail: deleting either call, moving one
/// after the seal it protects, gating one behind a condition, or adding a third
/// sealing exit without one.
#[test]
fn every_frame_seal_is_preceded_by_the_silent_outcome_sweep() {
    const SOURCE: &str = include_str!("../attach_log.rs");
    let lines: Vec<&str> = SOURCE.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with("fn build_compact_extraction_gate_records("))
        .expect("the gate function is in this file");
    let end = start
        + 1
        + lines[start + 1..]
            .iter()
            .position(|line| *line == "}")
            .expect("the gate function ends at a column-zero brace");

    // Brace depth relative to the function body, computed off CODE only: the
    // same string-literal stripper the frame-scoped status scan uses, so a `{}`
    // inside a reason string cannot move it.
    let mut depth = 0i32;
    let mut swept_at: Option<i32> = None;
    let mut seals = 0usize;
    let mut sweeps = 0usize;
    for line in &lines[start..end] {
        let trimmed = line.trim_start();
        let code = super::tests::without_string_literals(trimmed);
        // The depth a statement sits AT is the depth left after this line's own
        // closers, before its openers.
        let depth_here = depth - code.matches('}').count() as i32;
        if trimmed.starts_with("substantiate_silent_domain_outcomes(") {
            swept_at = Some(depth_here);
            sweeps += 1;
        }
        if trimmed.starts_with("append_compact_frame_end_record(") {
            seals += 1;
            assert_eq!(
                swept_at,
                Some(depth_here),
                "frame seal number {seals} in build_compact_extraction_gate_records is not \
                 preceded by substantiate_silent_domain_outcomes at the same brace depth \
                 (seal depth {depth_here}, last sweep {swept_at:?}). A sweep nested deeper is \
                 conditional, and a migrated domain that published a failing PERF word without \
                 saying why would refuse that whole hour."
            );
            swept_at = None;
        }
        depth = depth - code.matches('}').count() as i32 + code.matches('{').count() as i32;
    }
    // The scan must have found the thing it is policing. A renamed function or
    // a changed call spelling would otherwise make this pass vacuously.
    assert!(
        seals >= 2,
        "expected both sealing exits of the gate function, found {seals}"
    );
    assert!(sweeps >= seals, "found {sweeps} sweep(s) for {seals} seal(s)");
}

/// The reserved predicate the sweep publishes is a REAL one, constructible from
/// the contract's table, and distinct from the frame finding it replaces.
///
/// `migrated_site_published_outcome` is deliberately absent from
/// `READ_PREDICATES` -- it is the completeness walk's own predicate, not a check
/// a producer runs -- so an outcome that tried to carry it would silently
/// degrade to `Predicate::undeclared()` and the countable signal would be lost
/// in exactly the way that looks fine in review.
///
/// Production change that makes this fail: pointing the sweep at the frame
/// finding's code, or at any code not declared in the table.
#[test]
fn the_substantiated_outcome_carries_a_typed_countable_predicate() {
    let domain_id = COMPACT_DOMAIN_AIR_VARIANT_RESERVES;
    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
    );
    let (records, _) = assembled_frame(&[(domain_id, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    assert_eq!(
        substantiate_silent_domain_outcomes(&records, &mut outcomes, 0),
        1
    );
    let outcome = compact_contract::domain_outcome::failure_diagnosis(&outcomes, domain_id)
        .expect("the sweep records a failure");
    assert!(
        !outcome.predicate.is_undeclared(),
        "the substantiated outcome fell back to the reserved undeclared predicate"
    );
    assert_eq!(
        outcome.predicate.code.get(),
        compact_contract::domain_outcome::READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD
    );
    assert_eq!(outcome.predicate.name, "outcome_substantiated_from_record");
    // ...and it is a code a consumer can count, which is the whole reason it
    // exists rather than being left as prose in the reason string.
    assert_eq!(
        compact_contract::domain_outcome::read_predicate_name(
            compact_contract::domain_outcome::READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD
        ),
        Some("outcome_substantiated_from_record")
    );
    assert_ne!(
        compact_contract::domain_outcome::READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD,
        compact_contract::domain_outcome::READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME,
        "the per-domain signal and the frame finding must stay separable"
    );
}
