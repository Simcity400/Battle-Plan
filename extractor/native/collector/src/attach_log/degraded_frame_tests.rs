use super::*;

const TEST_HOUR: i32 = 60_831_436;

fn outcomes_for(
    failing_domains: &[(u16, u32)],
) -> compact_contract::domain_outcome::DomainOutcomeLedger {
    use compact_contract::domain_outcome::{
        read_predicate, AnswerExtent, ClockPresence, DomainOutcome, DomainOutcomeLedger,
        ReadCompletion, READ_PREDICATE_PAYLOAD_ENCODED,
    };
    let mut ledger = DomainOutcomeLedger::new();
    for (domain_id, status_flags) in failing_domains {
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
    let (notice_sequence, notice_domain, notice_type, notice_payload) =
        walked[walked.len() - 2].clone();
    assert_eq!(notice_domain, compact_contract::COMPACT_DOMAIN_FRAME);
    assert_eq!(
        notice_type,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_DEGRADED
    );
    let (frame_end_sequence, _, frame_end_type, _) = walked[walked.len() - 1];
    assert_eq!(frame_end_type, COMPACT_RECORD_TYPE_FRAME_END);
    assert_eq!(notice_sequence, frame_end_sequence_before);
    assert_eq!(frame_end_sequence, notice_sequence + 1);
    assert_eq!(next_sequence, frame_end_sequence + 1);

    let notice =
        compact_contract::frame_degraded::FrameDegradedRecordV2::decode(&notice_payload).unwrap();
    assert_eq!(notice.degraded_hour, TEST_HOUR);
    assert_eq!(notice.declared_domain_ids(), vec![29, 47]);
    assert!(notice
        .domains
        .iter()
        .all(|domain| !domain.reason.is_empty()));
}

#[test]
fn d65_and_d66_reader_failures_declare_their_own_outcome() {
    for domain_id in [
        COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE,
        COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE,
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
            findings
                .iter()
                .all(|finding| finding.frame_finding.is_none()),
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
        // TWO BINDING FORMS, and the second is not a loophole.
        //
        // Most sites write the published word straight back onto the read
        // (`read.status_flags = ...`), which makes "the word the record
        // publishes" and "the word the outcome derived" the same storage by
        // construction. The seven domains whose DATA payload a worker built
        // destructure it IMMUTABLY -- `let (read, published_payload) =
        // published.split();`, the shape
        // `the_seven_converted_domains_are_wired_through_the_publishing_helper`
        // pins -- so they take the word into a LOCAL instead. Accepting only
        // the assignment form would push those domains to choose between two
        // laws.
        //
        // The `let` form is checked harder BELOW rather than trusted: the
        // local it binds must be the one the PERF record's `status_flags`
        // field reads, so a site cannot derive one word and publish another.
        let trimmed = line.trim();
        let local = trimmed
            .strip_prefix("let ")
            .and_then(|rest| rest.strip_suffix(" = record_reader_status_outcome("));
        if !trimmed.starts_with("read.status_flags = record_reader_status_outcome(")
            && local.is_none()
        {
            continue;
        }
        let domain = lines[start + offset + 1..(start + offset + 5).min(end)]
            .iter()
            .map(|line| line.trim().trim_end_matches(','))
            .find(|line| line.starts_with("COMPACT_DOMAIN_"))
            .expect("the call names the domain it speaks for");
        if let Some(local) = local {
            // Bounded to THIS domain's block: from the call to the start of
            // the next `if let Some(..)` arm, so a neighbour's field cannot
            // satisfy it.
            let block_end = lines[start + offset + 1..end]
                .iter()
                .position(|line| line.starts_with("    if let Some("))
                .map(|at| start + offset + 1 + at)
                .unwrap_or(end);
            let published = format!("status_flags: {local},");
            assert!(
                lines[start + offset..block_end]
                    .iter()
                    .any(|line| line.trim() == published),
                "{domain} takes the derived word into `{local}` and must publish THAT \
                 local; deriving one word and publishing another is exactly the \
                 contradiction this pass exists to remove"
            );
        }
        routed.push(domain);
    }
    routed.sort_unstable();
    assert_eq!(
        routed,
        vec![
            "COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE",
            "COMPACT_DOMAIN_AIR_SUPERIORITY_INPUTS",
            "COMPACT_DOMAIN_AIR_VARIANT_RESERVES",
            "COMPACT_DOMAIN_AIR_WING_LOADOUTS",
            "COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE",
            "COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE",
            "COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE",
            "COMPACT_DOMAIN_COUNTRY_IDENTITY",
            "COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE",
            "COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE",
            "COMPACT_DOMAIN_DELAYED_EVENTS_LIVE",
            "COMPACT_DOMAIN_MARKET_TRADE_LIVE",
            "COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE",
            "COMPACT_DOMAIN_PROJECTS",
            "COMPACT_DOMAIN_RAID_INSTANCES_LIVE",
            "COMPACT_DOMAIN_SHIPS",
            "COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE",
            "COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE",
        ],
        "every emit site that publishes a reader-computed status word must substantiate it; \
         D36 and D38 joined D18/D65/D66/D91 when lane LC closed the silent-PERF class, D43 \
         joined them when the release-candidate capture showed all 324 of its frames \
         substantiated from the record instead of by their producer, D29/D42/D64/D93 \
         joined on rc_vanilla4, which carried the stand-in on 719, 74, 53 and 4 frames, \
         D67 joined on rc_vanilla5, which carried it on 2, and D32/D44/D54/D92/D96 \
         joined on e2e_03, which carried it on 1, 10 and 4 frames and, for D96 under the \
         reserved undeclared code, on 2, and D41 joined on e2e_05, which carried it on 65 \r
         consecutive hours"
    );
}

#[test]
fn record_reader_status_outcome_is_total_over_the_bits_it_claims() {
    let domain_id = COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE;
    let mut bit = 1u32;
    let mut covered = 0u32;
    while bit != 0 {
        if READER_OWNED_FAILURE_BITS & bit != 0 {
            let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
            let published = record_reader_status_outcome(
                &mut outcomes,
                domain_id,
                0,
                bit,
                String::new,
                Vec::new(),
            );
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

    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    record_read_succeeded(
        &mut outcomes,
        domain_id,
        compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
    );
    let published =
        record_reader_status_outcome(&mut outcomes, domain_id, 0, 0, String::new, Vec::new());
    assert_eq!(published, 0);
    assert!(!outcomes
        .get(domain_id)
        .expect("success stands")
        .is_failure());

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

    for (domain_id, status) in [
        (
            COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE,
            d66.status_flags,
        ),
        (
            COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE,
            d65.status_flags,
        ),
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
            findings
                .iter()
                .all(|finding| finding.frame_finding.is_none()),
            "domain {domain_id} under a profile mismatch became a frame fault: {:?}",
            findings
                .iter()
                .map(|finding| &finding.text)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_fully_migrated_site_that_publishes_failure_after_only_success_still_refuses() {
    let domain_id = COMPACT_DOMAIN_RESOLVED_TOOLTIP_INSTANCES_LIVE;
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

#[test]
fn a_tolerated_partial_leaves_the_frame_complete_and_produces_no_notice() {
    for tolerance in compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES {
        let (records, _) =
            assembled_frame(&[(18, 0), (tolerance.domain_id, tolerance.tolerated_flags)]);
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

#[test]
fn a_frame_level_fault_still_refuses_the_whole_frame() {
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

#[test]
fn a_framing_fault_refuses_even_when_per_domain_faults_could_have_degraded() {
    let (records, _) = assembled_frame(&[(18, 0), (29, COMPACT_STATUS_PARTIAL_DATA)]);
    let outcomes = outcomes_for(&[(29, COMPACT_STATUS_PARTIAL_DATA)]);
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

#[test]
fn a_failing_domain_with_no_diagnostic_units_is_named_undiagnosed_not_blank() {
    let candidates: Vec<u16> = compact_contract::ASSIGNED_COMPACT_DOMAINS
        .iter()
        .filter(|descriptor| descriptor.schedulable)
        .map(|descriptor| descriptor.id)
        .filter(|id| !MIGRATED_OUTCOME_DOMAINS.contains(id))
        .filter(|id| compact_contract::expected_partial_tolerance(*id).is_none())
        .collect();
    assert!(
        candidates.len() >= 2,
        "this test needs two domains whose producer genuinely declares nothing (unmigrated, no \
         partial tolerance); the contract now offers {}. If migration has reached them all, the \
         silence this test describes is gone and the test should be DELETED, not weakened",
        candidates.len()
    );
    let silent_domain = candidates[0];
    let other_silent_domain = candidates[1];

    let (records, _) = assembled_frame(&[(silent_domain, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR, &no_outcomes());
    let FrameCompletenessVerdict::Degraded(domains) = classify_frame_completeness(&findings) else {
        panic!("a lone per-domain fault must degrade");
    };
    // here is what would have re-deadlocked the frame.
    assert!(
        !domains[0].reason.is_empty(),
        "a silent domain must still carry a reason"
    );
    let (other_records, _) =
        assembled_frame(&[(other_silent_domain, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    let other_findings =
        compact_frame_completeness_findings(&other_records, TEST_HOUR, &no_outcomes());
    let FrameCompletenessVerdict::Degraded(other_domains) =
        classify_frame_completeness(&other_findings)
    else {
        panic!("a lone per-domain fault must degrade");
    };
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
    let strip = |domain_id: u16, reason: &str| {
        reason
            .strip_prefix(&format!("domain {domain_id}: "))
            .unwrap_or(reason)
            .to_string()
    };
    assert_eq!(
        strip(silent_domain, &domains[0].reason),
        strip(other_silent_domain, &other_domains[0].reason),
        "every silent domain must be named by the SAME marker, not a per-domain one"
    );
    // And it is publishable: a silent domain must not be able to re-deadlock a
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

#[test]
fn every_degraded_status_withholds_its_delta_baseline_for_every_domain() {
    let tolerance_domains: Vec<u16> = compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES
        .iter()
        .map(|tolerance| tolerance.domain_id)
        .collect();
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

#[test]
fn one_domain_objecting_twice_is_a_frame_fault_not_a_merged_entry() {
    let (records, _) = assembled_frame(&[
        (29, COMPACT_STATUS_PARTIAL_DATA),
        (29, COMPACT_STATUS_NO_CLOCK),
    ]);
    let findings = compact_frame_completeness_findings(
        &records,
        TEST_HOUR,
        &outcomes_for(&[(29, COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_NO_CLOCK)]),
    );
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

#[test]
fn a_notice_that_cannot_be_spliced_fails_closed() {
    let degraded = vec![
        compact_contract::frame_degraded::FrameDegradedDomain::from_status(
            29,
            COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::frame_degraded::FrameDegradedDiagnosis::undeclared(
                compact_contract::diagnostic_channel::Authority::SelfDomain,
                "d29_invalid_air_wing_rows=3",
            ),
        )
        .unwrap(),
    ];

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

    let (mut records, mut next_sequence) = assembled_frame(&[(29, COMPACT_STATUS_PARTIAL_DATA)]);
    assert!(insert_frame_degraded_notice_before_frame_end(
        &mut records,
        &mut next_sequence,
        TEST_HOUR + 1,
        degraded,
    )
    .is_err());
}

#[test]
fn a_full_roster_degraded_summary_reaches_the_log_whole() {
    use compact_contract::worker_error_record as record_format;

    let reason_len = compact_contract::frame_degraded::FRAME_DEGRADED_MAX_REASON_LEN;
    let last_domain = 99u16;
    let summary = (0..73u16)
        .map(|index| {
            let domain = if index == 72 { last_domain } else { index };
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

    let pid = unsafe { GetCurrentProcessId() };
    let creation = current_process_creation_filetime().unwrap_or(0);
    let header = record_format::owned_header(pid, creation);
    let suffix = record_format::owned_suffix(pid);
    let mut carried = String::new();
    for record in &records {
        let line = record
            .strip_suffix('\n')
            .expect("record ends with a newline");
        let body = record_format::owned_body(line.as_bytes(), &header, &suffix)
            .expect("every chunk must be an intact record the host can own");
        carried.push_str(&String::from_utf8_lossy(body));
    }

    let tail = format!(
        "d{last_domain}:{:#010x}[",
        COMPACT_STATUS_SOURCE_UNAVAILABLE
    );
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

const EVERY_READER_OWNED_FAILURE_BIT: &[u32] = &[
    COMPACT_STATUS_SOURCE_UNAVAILABLE,
    COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_WORKER_PANIC,
    COMPACT_STATUS_NO_CLOCK,
    COMPACT_STATUS_CONTROL_INVALID,
];

#[test]
fn no_migrated_domain_can_publish_a_failing_perf_word_without_an_outcome() {
    let mut exercised = 0usize;
    for domain_id in MIGRATED_OUTCOME_DOMAINS.iter().copied() {
        for bit in EVERY_READER_OWNED_FAILURE_BIT.iter().copied() {
            let failed =
                compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
                    domain_id, bit,
                );
            if failed == 0 {
                continue;
            }
            let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
            record_read_succeeded(
                &mut outcomes,
                domain_id,
                compact_contract::domain_outcome::READ_PREDICATE_WORKER_COMPLETED,
            );
            let (records, _) = assembled_frame(&[(domain_id, bit)]);

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
                after
                    .iter()
                    .map(|finding| &finding.text)
                    .collect::<Vec<_>>()
            );
            assert!(
                !after
                    .iter()
                    .any(|finding| finding
                        .frame_finding
                        .as_ref()
                        .is_some_and(
                            |frame| frame.predicate.name == "published_status_matches_outcome"
                        )),
                "domain {domain_id} with status {bit:#010x} made the record and the diagnosis \
                 disagree: {:?}",
                after
                    .iter()
                    .map(|finding| &finding.text)
                    .collect::<Vec<_>>()
            );
            let FrameCompletenessVerdict::Degraded(degraded) = classify_frame_completeness(&after)
            else {
                panic!(
                    "domain {domain_id} with status {bit:#010x} must become a per-domain \
                     degradation, not a verdict about the frame"
                );
            };
            assert_eq!(degraded.len(), 1);
            assert_eq!(degraded[0].domain_id, domain_id);
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
    assert!(
        exercised >= MIGRATED_OUTCOME_DOMAINS.len(),
        "only {exercised} (domain, bit) cases ran across {} declared domains",
        MIGRATED_OUTCOME_DOMAINS.len()
    );
}

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

#[test]
fn a_frame_prologue_bit_alone_is_left_to_refuse_the_frame() {
    let domain_id = COMPACT_DOMAIN_AIR_VARIANT_RESERVES;
    assert_ne!(
        compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
        ),
        0,
        "a bare bridge bit must stay outside d47's expected-partial tolerance, or its \
         scheduled reader stops normalising the word this sweep cannot substantiate"
    );
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

    let mut depth = 0i32;
    let mut swept_at: Option<i32> = None;
    let mut seals = 0usize;
    let mut sweeps = 0usize;
    for line in &lines[start..end] {
        let trimmed = line.trim_start();
        let code = super::tests::without_string_literals(trimmed);
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
    assert!(
        seals >= 2,
        "expected both sealing exits of the gate function, found {seals}"
    );
    assert!(
        sweeps >= seals,
        "found {sweeps} sweep(s) for {seals} seal(s)"
    );
}

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
