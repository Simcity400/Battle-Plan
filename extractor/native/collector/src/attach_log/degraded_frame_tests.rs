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
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR);
    assert_eq!(
        findings.len(),
        2,
        "both failing domains must object, and the clean one must not"
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

/// A domain inside its expected-partial tolerance is NOT a degradation. The
/// frame stays complete and no notice is produced -- the two states must never
/// collapse into one another.
#[test]
fn a_tolerated_partial_leaves_the_frame_complete_and_produces_no_notice() {
    for tolerance in compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES {
        let (records, _) = assembled_frame(&[(18, 0), (tolerance.domain_id, tolerance.tolerated_flags)]);
        let findings = compact_frame_completeness_findings(&records, TEST_HOUR);
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
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR + 1);
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
    let findings = compact_frame_completeness_findings(&headless, TEST_HOUR);
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
    let findings = compact_frame_completeness_findings(&bad_frame_end, TEST_HOUR);
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
    // Walking against the wrong hour makes EVERY record raise a framing
    // finding, while domain 29 also raises its per-domain objection. The
    // per-domain half must not be able to carry the frame.
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR + 1);
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
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR);
    assert!(!findings.is_empty());
    assert!(
        matches!(
            classify_frame_completeness(&findings),
            FrameCompletenessVerdict::Refused
        ),
        "an unreadable status must refuse, not be guessed at"
    );
}

/// A failing domain that emitted no diagnostic units still gets a typed reason.
///
/// Some scheduled domains attach no units at all. Their failures used to arrive
/// as a bare status bit, and an entry with an empty reason is refused by the
/// codec -- so without one shared name for the condition, a silent domain would
/// have made its whole frame unpublishable. The shared name makes the silence
/// countable instead of fatal, and it is the SAME name for every domain.
#[test]
fn a_failing_domain_with_no_diagnostic_units_is_named_undiagnosed_not_blank() {
    let (records, _) = assembled_frame(&[(29, COMPACT_STATUS_SOURCE_UNAVAILABLE)]);
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR);
    let FrameCompletenessVerdict::Degraded(domains) = classify_frame_completeness(&findings) else {
        panic!("a lone per-domain fault must degrade");
    };
    assert_eq!(
        domains[0].reason,
        compact_contract::frame_degraded::FRAME_DEGRADED_REASON_UNDIAGNOSED
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
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR);
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
    let findings = compact_frame_completeness_findings(&records, TEST_HOUR);
    assert_eq!(findings.len(), 2);
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
