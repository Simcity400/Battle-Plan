use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn failed_build_finishes_prior_publication_once_and_recycles_current_storage() {
    let published = Arc::new(AtomicUsize::new(0));
    let observer = Arc::clone(&published);
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(&[7; 19]);
    let address = out.as_ptr();
    let capacity = out.capacity();
    let mut buffers = FrameBuffers::default();
    let error = finish_failed_compact_frame_build(
        "D20 control output storage unavailable before acquisition",
        Some(Box::new(move || {
            observer.fetch_add(1, Ordering::SeqCst);
        })),
        out,
        &mut buffers,
    );
    assert_eq!(published.load(Ordering::SeqCst), 1);
    assert_eq!(
        error,
        "D20 control output storage unavailable before acquisition"
    );
    let recycled = buffers.take_record_buffer();
    assert!(recycled.is_empty());
    assert_eq!(recycled.as_ptr(), address);
    assert_eq!(recycled.capacity(), capacity);
}

#[test]
fn output_storage_refusal_has_a_distinct_encodable_frame_finding() {
    let reason = "D20 control output storage unavailable before acquisition";
    let finding = framing_finding_with(
        FRAME_PREDICATE_OUTPUT_STORAGE_AVAILABLE,
        "frame_output_storage_available",
        reason.into(),
    );
    assert!(finding.offender.is_none());
    let record = compact_contract::frame_refusal::FrameRefusalRecordV2 {
        refused_hour: 60_832_384,
        offenders: Vec::new(),
        frame_findings: finding.frame_finding.into_iter().collect(),
    };
    let decoded = compact_contract::frame_refusal::FrameRefusalRecordV2::decode(
        &record.encode().expect("storage refusal encodes"),
    )
    .expect("storage refusal decodes");
    assert_eq!(decoded.frame_findings.len(), 1);
    assert_eq!(decoded.frame_findings[0].reason, reason);
    assert_ne!(
        FRAME_PREDICATE_OUTPUT_STORAGE_AVAILABLE,
        FRAME_PREDICATE_RECORD_FRAMING
    );
}

#[test]
fn build_refusal_settles_prior_publications_before_releasing_current_custody() {
    let source = include_str!("../attach_log.rs");
    let start = source
        .find("let build_result =")
        .expect("production build result");
    let caller = &source[start..];
    let end = caller
        .find("if let Some(frame) = finalized_render_frame")
        .unwrap();
    let caller = &caller[..end];
    let resolve = caller.find("resolve_staged_publications(").unwrap();
    let dispose = caller.find("dispose_staged_publications(").unwrap();
    let control = caller.find("deferred_gap_reservation.take()").unwrap();
    let refusal = caller.find("match build_result").unwrap();
    assert!(resolve < dispose && dispose < control && control < refusal);
    let published = &caller[caller
        .find("Ok(FrameRefusalPublication::Published)")
        .unwrap()..];
    let restore = published
        .find("restore_country_trackers_after_refused_frame(")
        .unwrap();
    let drop_staged = published.find("drop(staged_trackers)").unwrap();
    let invalidate = published
        .find("invalidate_published_division_baseline(")
        .unwrap();
    let reject = published
        .find("runtime.record_frame_rejected(request)")
        .unwrap();
    assert!(restore < drop_staged && drop_staged < invalidate && invalidate < reject);
    assert!(published[..reject].contains("domain_data_repeat_gate.note_gap_domain(domain)"));
    assert!(caller.contains("let refusal_owner = hourly_service_capture::current_owner()"));
    assert!(!caller.contains("CompactFramePhaseTiming::default()"));

    let builder = &source[source
        .find("fn build_compact_extraction_gate_records(")
        .unwrap()..];
    let reserve = builder
        .find("let mut division_storage_gap_reservation =")
        .unwrap();
    let finish = builder[reserve..]
        .find("finish_failed_compact_frame_build(")
        .unwrap()
        + reserve;
    let baseline = builder[reserve..]
        .find("retain_baseline_membership(")
        .unwrap()
        + reserve;
    assert!(reserve < finish && finish < baseline);
    assert!(builder[..reserve].contains("Result<(Vec<u8>, u64, CompactFramePhaseTiming), String>"));
    assert!(!builder[reserve..baseline].contains(".expect("));
}
