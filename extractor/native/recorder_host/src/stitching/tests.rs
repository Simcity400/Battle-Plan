use super::*;
use std::collections::BTreeMap;

fn verified(scheme: u16, key: &[u8]) -> CampaignIdentityResolution {
    CampaignIdentityResolution::Verified {
        scheme_major: scheme,
        campaign_key: key.to_vec(),
        evidence_digest: [0u8; 32],
    }
}

fn ticket(key: &[u8], order: u64) -> AppendTicket {
    AppendTicket {
        campaign_key: key.to_vec(),
        play_order: order,
        session_nonce: format!("nonce-{order}"),
        process_start_identity: format!("pid-{order}"),
    }
}

fn stitch_inputs() -> StitchRequiredInputs {
    StitchRequiredInputs {
        schema_version: "schema-a".into(),
        fold_version: "world-fold-test-v1".into(),
        canonical_writer_version: "snapshot-canon-test-v1".into(),
        packer_codec_version: "zip-test-v1".into(),
        created_utc: "2026-07-13T00:00:00Z".into(),
        compression_method: "zstd".into(),
        compression_parameters: BTreeMap::from([("level".into(), "default".into())]),
        entry_order: STITCH_ENTRY_ORDER.into(),
    }
}

fn snap_seg(
    play_order: u64,
    ord: u32,
    key: &[u8],
    range: std::ops::RangeInclusive<i64>,
) -> SegmentInput {
    let hours: Vec<i64> = range.collect();
    let append_ticket = ticket(key, play_order);
    let epoch_token = format!("epoch-{play_order}-{ord}");
    let owner = ObservationOwner {
        ticket: append_ticket.clone(),
        in_session_ordinal: ord,
        epoch_token: epoch_token.clone(),
    };
    let mut coverage = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for (index, h) in hours.iter().copied().enumerate() {
        let disposition = CoverageDisposition::Snapshot {
            sha256: format!("{h:064x}"),
            date: Some(format!("1936.1.1.{h}")),
        };
        coverage.insert(h, disposition.clone());
        observations.insert(
            h,
            vec![HourObservation {
                observation_id: format!("obs-{play_order}-{ord}-{h}"),
                owner: owner.clone(),
                record_seq_bounds: ((index as u64) * 2, (index as u64) * 2 + 1),
                byte_bounds: ((index as u64) * 100, (index as u64 + 1) * 100),
                disposition,
            }],
        );
    }
    let source_record_end = (hours.len() as u64) * 2 - 1;
    let source_byte_end = (hours.len() as u64) * 100;
    let stored_digest = format!("{:064x}", play_order + 1);
    let decoded_digest = format!("{:064x}", play_order + 1_000);
    let source_path = format!("sessions/p{play_order:08}/collector_stream.h4cr");
    let participation = SourceParticipation {
        ticket: append_ticket.clone(),
        scheme_major: 1,
        identity_evidence_digest: [0u8; 32],
        record_seq_bounds: (0, source_record_end),
        byte_bounds: (0, source_byte_end),
    };
    SegmentInput {
        play_order,
        in_session_ordinal: ord,
        identity: verified(1, key),
        ticket: Some(append_ticket),
        compatibility: CompatibilityResolution::Proven {
            evidence_id: format!("compat-evidence-{play_order}"),
            compatibility_class: "test-decoder-v1".into(),
            build_exe_sha256: "ab".repeat(32),
            modset_id: "modset-a".into(),
            schema_version: "schema-a".into(),
            static_catalog_digest: "cd".repeat(32),
        },
        transition_coverage: BTreeMap::new(),
        coverage,
        observations,
        raw_source: Some(RawSourceDescriptor {
            session_nonce: format!("nonce-{play_order}"),
            process_start_identity: format!("pid-{play_order}"),
            source_path: source_path.clone(),
            stored_digest: stored_digest.clone(),
            decoded_digest: decoded_digest.clone(),
            record_seq_bounds: (0, source_record_end),
            byte_bounds: (0, source_byte_end),
            storage: RawSourceStorage::Standalone,
            participations: vec![participation],
        }),
        provenance: SegmentProvenance {
            session_nonce: format!("nonce-{play_order}"),
            process_start_identity: format!("pid-{play_order}"),
            epoch_token,
            stream_major: 2,
            build_exe_sha256: Some("ab".repeat(32)),
            modset_id: Some("modset-a".into()),
            schema_version: Some("schema-a".into()),
            static_catalog_digest: Some("cd".repeat(32)),
            source_path: Some(source_path),
            source_stored_digest: Some(stored_digest),
            source_decoded_digest: Some(decoded_digest),
            record_seq_bounds: Some((0, source_record_end)),
            byte_bounds: Some((0, source_byte_end)),
            ..Default::default()
        },
    }
}

fn stitched(outcome: StitchOutcome) -> StitchedPlan {
    match outcome {
        StitchOutcome::Stitched(p) => p,
        StitchOutcome::Separate { reason } => panic!("expected Stitched, got Separate: {reason}"),
    }
}

fn extend_source_bounds(segment: &mut SegmentInput, record_end: u64, byte_end: u64) {
    segment.provenance.record_seq_bounds = Some((0, record_end));
    segment.provenance.byte_bounds = Some((0, byte_end));
    let source = segment.raw_source.as_mut().unwrap();
    source.record_seq_bounds = (0, record_end);
    source.byte_bounds = (0, byte_end);
    let participation = source
        .participations
        .iter_mut()
        .find(|participation| participation.ticket == *segment.ticket.as_ref().unwrap())
        .unwrap();
    participation.record_seq_bounds = (0, record_end);
    participation.byte_bounds = (0, byte_end);
}

fn main_hours(p: &StitchedPlan) -> Vec<i64> {
    p.supersede.main.keys().copied().collect()
}

const K: &[u8] = b"campaign-A";

#[test]
fn rollback_middle_supersedes_and_retains() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let s1 = snap_seg(1, 0, K, 80..=90);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));

    assert_eq!(main_hours(&plan), (0..=90).collect::<Vec<_>>());
    assert_eq!(plan.supersede.main_head, Some(90));
    assert_eq!(plan.supersede.main[&79].segment_id, 0);
    assert_eq!(plan.supersede.main[&80].segment_id, 1);
    assert_eq!(plan.supersede.main[&90].segment_id, 1);
    assert!(!plan.supersede.main.contains_key(&91));

    let sup: Vec<_> = plan
        .supersede
        .superseded
        .iter()
        .filter(|s| s.segment_id == 0)
        .collect();
    assert_eq!(sup.len(), 1, "one coalesced superseded span");
    assert_eq!((sup[0].start_hour, sup[0].end_hour), (80, 100));
    assert_eq!(sup[0].superseded_by, 1);
}

#[test]
fn overlap_at_start_boundary() {
    let s0 = snap_seg(0, 0, K, 0..=50);
    let s1 = snap_seg(1, 0, K, 50..=60);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));
    assert_eq!(main_hours(&plan), (0..=60).collect::<Vec<_>>());
    assert_eq!(plan.supersede.main[&50].segment_id, 1);
    assert!(plan.supersede.superseded.iter().any(|s| s.segment_id == 0
        && s.start_hour <= 50
        && 50 <= s.end_hour
        && s.superseded_by == 1));
}

#[test]
fn overlap_at_end_extends_past_old_head() {
    let s0 = snap_seg(0, 0, K, 0..=40);
    let s1 = snap_seg(1, 0, K, 30..=80);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));
    assert_eq!(main_hours(&plan), (0..=80).collect::<Vec<_>>());
    assert_eq!(plan.supersede.main_head, Some(80));
    assert_eq!(plan.supersede.main[&29].segment_id, 0);
    assert_eq!(plan.supersede.main[&30].segment_id, 1);
    assert_eq!(plan.supersede.main[&80].segment_id, 1);
}

#[test]
fn relaunch_continuation_extends_main() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let s1 = snap_seg(1, 0, K, 80..=90);
    let s2 = snap_seg(2, 0, K, 91..=120);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1, s2]));
    assert_eq!(main_hours(&plan), (0..=120).collect::<Vec<_>>());
    assert_eq!(plan.supersede.main[&79].segment_id, 0);
    assert_eq!(plan.supersede.main[&85].segment_id, 1);
    assert_eq!(plan.supersede.main[&100].segment_id, 2);
    assert_eq!(plan.supersede.main_head, Some(120));
}

#[test]
fn cross_session_gap_is_explicit_not_invented() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let mut s2 = snap_seg(1, 0, K, 110..=120);
    for hour in 101..=109 {
        s2.transition_coverage.insert(
            hour,
            CoverageDisposition::Gap {
                reason: "session_gap".into(),
                evidence_id: "transition-0-1".into(),
            },
        );
    }
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s2]));
    for h in 101..=109 {
        assert!(matches!(&plan.supersede.main[&h].disposition,
            CoverageDisposition::Gap { reason, evidence_id }
                if reason == "session_gap" && evidence_id == "transition-0-1"));
    }
    assert!(plan.supersede.main.contains_key(&100));
    assert!(plan.supersede.main.contains_key(&110));
    assert_eq!(plan.supersede.main_head, Some(120));
    assert_eq!(
        plan.main_index_value()["coverage"],
        json!([{
            "kind":"gap", "reason":"session_gap", "evidence_id":"transition-0-1",
            "segment_id":1, "start_hour":101, "end_hour":109,
            "owner": plan.segments[1].input.observation_owner().unwrap().to_value()
        }])
    );
}

#[test]
fn refuse_missing_cross_session_transition_coverage() {
    match plan_stitch(
        "g",
        stitch_inputs(),
        vec![snap_seg(0, 0, K, 0..=100), snap_seg(1, 0, K, 110..=120)],
    ) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("transition coverage")),
        other => panic!("missing transition ledger must fail closed, got {other:?}"),
    }
}

#[test]
fn refuse_non_proven_compatibility_results() {
    let cases = [
        CompatibilityResolution::Missing {
            reason: "absent".into(),
        },
        CompatibilityResolution::Conflict {
            reason: "clash".into(),
        },
        CompatibilityResolution::Uncertain {
            reason: "unknown".into(),
        },
    ];
    for compatibility in cases {
        let mut s = snap_seg(0, 0, K, 0..=1);
        s.compatibility = compatibility;
        assert!(matches!(
            plan_stitch("g", stitch_inputs(), vec![s]),
            StitchOutcome::Separate { .. }
        ));
    }
}

#[test]
fn refuse_compatibility_proof_binding_mismatch() {
    let mut s = snap_seg(0, 0, K, 0..=1);
    s.provenance.modset_id = Some("foreign-modset".into());
    match plan_stitch("g", stitch_inputs(), vec![s]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("source bindings")),
        other => panic!("mismatched proof must fail closed, got {other:?}"),
    }
}

#[test]
fn later_failed_hour_is_a_main_gap_over_older_snapshot() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let mut s1 = snap_seg(1, 0, K, 80..=90);
    s1.coverage.insert(
        85,
        CoverageDisposition::CaptureFailed {
            reason: "fold_failed".into(),
            evidence_id: "ev-85".into(),
        },
    );
    let observation = &mut s1.observations.get_mut(&85).unwrap()[0];
    observation.observation_id = "ev-85".into();
    observation.disposition = s1.coverage[&85].clone();
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));
    let mh85 = &plan.supersede.main[&85];
    assert_eq!(mh85.segment_id, 1);
    assert!(
        !mh85.disposition.is_snapshot(),
        "later failed hour must be a gap"
    );
    let idx = plan.main_index_value();
    let snaps = idx["snapshots"].as_array().unwrap();
    assert!(snaps.iter().all(|e| e["hour_index"].as_i64() != Some(85)));
    let cov = idx["coverage"].as_array().unwrap();
    assert!(cov
        .iter()
        .any(|c| c["start_hour"].as_i64() == Some(85)
            && c["kind"].as_str() == Some("capture_failed")));
}

#[test]
fn main_index_dates_every_visible_snapshot_hour() {
    let plan = stitched(plan_stitch(
        "g",
        stitch_inputs(),
        vec![snap_seg(0, 0, K, 0..=2), snap_seg(1, 0, K, 3..=4)],
    ));
    let index = plan.main_index_value();
    let snapshots = index["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 5);
    for row in snapshots {
        let hour = row["hour_index"].as_i64().expect("hour_index");
        assert_eq!(
            row["date"].as_str(),
            Some(format!("1936.1.1.{hour}").as_str()),
            "visible hour {hour} must carry its index date: {row}"
        );
    }
}

#[test]
fn a_visible_snapshot_without_a_date_is_refused() {
    let mut undated = snap_seg(0, 0, K, 0..=0);
    undated
        .coverage
        .get_mut(&0)
        .unwrap()
        .clone_from(&CoverageDisposition::Snapshot {
            sha256: format!("{:064x}", 0),
            date: None,
        });
    let disposition = undated.coverage.get(&0).unwrap().clone();
    undated.observations.get_mut(&0).unwrap()[0].disposition = disposition;
    assert!(matches!(
        plan_stitch(
            "g",
            stitch_inputs(),
            vec![undated, snap_seg(1, 0, K, 1..=1)]
        ),
        StitchOutcome::Separate { .. }
    ));
}

#[test]
fn multi_reload_chain() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let s1 = snap_seg(1, 0, K, 90..=95);
    let s2 = snap_seg(2, 0, K, 50..=55);
    let s3 = snap_seg(3, 0, K, 52..=200);
    let plan = stitched(plan_stitch(
        "g",
        stitch_inputs(),
        vec![s0.clone(), s1.clone(), s2.clone(), s3.clone()],
    ));

    assert_eq!(plan.supersede.main[&49].segment_id, 0);
    assert_eq!(plan.supersede.main[&50].segment_id, 2);
    assert_eq!(plan.supersede.main[&51].segment_id, 2);
    assert_eq!(plan.supersede.main[&52].segment_id, 3);
    assert_eq!(plan.supersede.main[&200].segment_id, 3);
    assert_eq!(plan.supersede.main_head, Some(200));

    let inputs = [(&s0, 0u32), (&s1, 1), (&s2, 2), (&s3, 3)];
    for (seg, gid) in inputs {
        for (h, disp) in seg.coverage.iter() {
            if !disp.is_snapshot() {
                continue;
            }
            let visible_here = plan
                .supersede
                .main
                .get(h)
                .map(|mh| mh.segment_id == gid)
                .unwrap_or(false);
            if !visible_here {
                let covered = plan
                    .supersede
                    .superseded
                    .iter()
                    .any(|s| s.segment_id == gid && s.start_hour <= *h && *h <= s.end_hour);
                assert!(
                    covered,
                    "excluded snapshot seg {gid} hour {h} not in a rollback span"
                );
            }
        }
    }
}

#[test]
fn abandoned_future_never_resumed() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let s1 = snap_seg(1, 0, K, 20..=30);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));
    assert_eq!(plan.supersede.main_head, Some(30));
    for h in 31..=100 {
        assert!(
            !plan.supersede.main.contains_key(&h),
            "abandoned future hour {h} resumed"
        );
    }
}

#[test]
fn rollback_retention_roundtrip_via_serialization() {
    let s0 = snap_seg(0, 0, K, 0..=100);
    let s1 = snap_seg(1, 0, K, 80..=90);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));

    let segs = plan.segments_value();
    let seg0 = segs["segments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["segment_id"].as_i64() == Some(0))
        .unwrap();
    let sup = seg0["superseded_spans"].as_array().unwrap();
    assert_eq!(sup.len(), 1);
    assert_eq!(sup[0]["start_hour"].as_i64(), Some(80));
    assert_eq!(sup[0]["end_hour"].as_i64(), Some(100));
    assert_eq!(sup[0]["superseded_by"].as_i64(), Some(1));

    let pseg0 = plan.segments.iter().find(|p| p.segment_id == 0).unwrap();
    let idx0 = plan.segment_index_value(pseg0);
    let hours: Vec<i64> = idx0["hours"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["hour_index"].as_i64().unwrap())
        .collect();
    assert_eq!(
        hours,
        (0..=100).collect::<Vec<_>>(),
        "no retained hour dropped"
    );
    let h95 = idx0["hours"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["hour_index"].as_i64() == Some(95))
        .unwrap();
    assert_eq!(h95["kind"].as_str(), Some("snapshot"));
}

#[test]
fn visible_index_uses_segment_prefixed_filenames() {
    let s0 = snap_seg(0, 0, K, 0..=2);
    let s1 = snap_seg(1, 0, K, 1..=3);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, s1]));
    let files: Vec<String> = plan
        .visible_snapshot_files()
        .into_iter()
        .map(|(_, f)| f)
        .collect();
    assert!(files.contains(&"s00000000.000000000000.snapshot.json".to_string()));
    assert!(files.contains(&"s00000001.000000000001.snapshot.json".to_string()));
    assert!(files.contains(&"s00000001.000000000003.snapshot.json".to_string()));
    assert!(!files.contains(&"s00000000.000000000001.snapshot.json".to_string()));
}

#[test]
fn refuse_unbound_identity_missing() {
    let mut s = snap_seg(0, 0, K, 0..=10);
    s.identity = CampaignIdentityResolution::Missing {
        reason: "no epoch".into(),
    };
    match plan_stitch("g", stitch_inputs(), vec![s]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("Missing")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_ambiguous_and_conflict() {
    let mut a = snap_seg(0, 0, K, 0..=10);
    a.identity = CampaignIdentityResolution::Ambiguous {
        reason: "two".into(),
        candidate_count: 2,
    };
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![a]),
        StitchOutcome::Separate { .. }
    ));

    let mut c = snap_seg(0, 0, K, 0..=10);
    c.identity = CampaignIdentityResolution::Conflict {
        reason: "clash".into(),
    };
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![c]),
        StitchOutcome::Separate { .. }
    ));
}

#[test]
fn refuse_missing_ticket() {
    let mut s = snap_seg(0, 0, K, 0..=10);
    s.ticket = None;
    match plan_stitch("g", stitch_inputs(), vec![s]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("AppendTicket")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_mismatched_campaign_keys() {
    let s0 = snap_seg(0, 0, K, 0..=10);
    let s1 = snap_seg(1, 0, b"campaign-B", 5..=15);
    match plan_stitch("g", stitch_inputs(), vec![s0, s1]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("campaign keys differ")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_mixed_scheme_major() {
    let s0 = snap_seg(0, 0, K, 0..=10);
    let mut s1 = snap_seg(1, 0, K, 5..=15);
    s1.identity = verified(2, K);
    match plan_stitch("g", stitch_inputs(), vec![s0, s1]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("scheme_major")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_mixed_stream_major() {
    let s0 = snap_seg(0, 0, K, 0..=10);
    let mut s1 = snap_seg(1, 0, K, 5..=15);
    s1.provenance.stream_major = 3;
    match plan_stitch("g", stitch_inputs(), vec![s0, s1]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("stream major")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_duplicate_play_order_key() {
    let mut s0 = snap_seg(0, 0, K, 0..=10);
    let mut s1 = snap_seg(0, 0, K, 5..=15);
    s0.provenance.session_nonce = "nonce-A".into();
    s1.provenance.session_nonce = "nonce-B".into();
    s0.ticket = Some(AppendTicket {
        session_nonce: "nonce-A".into(),
        ..ticket(K, 0)
    });
    s1.ticket = Some(AppendTicket {
        session_nonce: "nonce-B".into(),
        ..ticket(K, 0)
    });
    match plan_stitch("g", stitch_inputs(), vec![s0, s1]) {
        StitchOutcome::Separate { reason } => {
            assert!(reason.contains("duplicate") || reason.contains("unprovable"))
        }
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_ticket_campaign_key_disagreement() {
    let mut s = snap_seg(0, 0, K, 0..=10);
    s.ticket = Some(ticket(b"other-key", 0));
    match plan_stitch("g", stitch_inputs(), vec![s]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("disagrees")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn refuse_every_ticket_field_disagreement() {
    let mutations: [(&str, fn(&mut AppendTicket)); 3] = [
        ("play_order", |t| t.play_order += 1),
        ("session_nonce", |t| t.session_nonce.push_str("-foreign")),
        ("process_start_identity", |t| {
            t.process_start_identity.push_str("-foreign")
        }),
    ];

    for (field, mutate) in mutations {
        let mut s = snap_seg(0, 0, K, 0..=10);
        mutate(s.ticket.as_mut().unwrap());
        match plan_stitch("g", stitch_inputs(), vec![s]) {
            StitchOutcome::Separate { reason } => assert!(
                reason.contains(field),
                "{field} mismatch refusal did not name the field: {reason}"
            ),
            other => panic!("{field} mismatch must fail closed, got {other:?}"),
        }
    }
}

#[test]
fn refuse_corrupt_coverage_hole() {
    let mut s = snap_seg(0, 0, K, 0..=10);
    s.coverage.remove(&5);
    s.observations.remove(&5);
    match plan_stitch("g", stitch_inputs(), vec![s]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("missing coverage")),
        other => panic!("expected Separate, got {other:?}"),
    }
}

#[test]
fn determinism_input_order_permutation() {
    let build = || {
        vec![
            snap_seg(2, 0, K, 91..=120),
            snap_seg(0, 0, K, 0..=100),
            snap_seg(1, 0, K, 80..=90),
        ]
    };
    let mut a = build();
    let mut b = build();
    a.reverse();
    b.rotate_left(1);
    let pa = stitched(plan_stitch("g", stitch_inputs(), a));
    let pb = stitched(plan_stitch("g", stitch_inputs(), b));
    assert_eq!(pa, pb, "plan is a pure function of the segment set");

    let fa = pa.to_container_files();
    let fb = pb.to_container_files();
    assert_eq!(fa, fb);
    assert_eq!(pa.stitch_input_digest, pb.stitch_input_digest);
    assert_eq!(pa.play_order_ledger_digest, pb.play_order_ledger_digest);
}

#[test]
fn determinism_input_change_moves_digest() {
    let base = vec![snap_seg(0, 0, K, 0..=100), snap_seg(1, 0, K, 80..=90)];
    let p_base = stitched(plan_stitch("g", stitch_inputs(), base));

    let changed = vec![snap_seg(0, 0, K, 0..=100), snap_seg(1, 0, K, 81..=90)];
    let p_changed = stitched(plan_stitch("g", stitch_inputs(), changed));
    assert_ne!(
        p_base.stitch_input_digest, p_changed.stitch_input_digest,
        "a cutover change must move the stitch input digest"
    );
}

#[test]
fn global_ordinals_scan_from_zero_in_play_order() {
    let mut same_session_0 = snap_seg(5, 0, K, 0..=10);
    let mut same_session_1 = snap_seg(5, 1, K, 10..=20);
    for observations in same_session_1.observations.values_mut() {
        for observation in observations {
            observation.record_seq_bounds.0 += 22;
            observation.record_seq_bounds.1 += 22;
            observation.byte_bounds.0 += 1_100;
            observation.byte_bounds.1 += 1_100;
        }
    }
    same_session_1.provenance.record_seq_bounds = Some((22, 43));
    same_session_1.provenance.byte_bounds = Some((1_100, 2_200));
    let source = same_session_0.raw_source.as_mut().unwrap();
    source.record_seq_bounds = (0, 43);
    source.byte_bounds = (0, 2_200);
    source.participations[0].record_seq_bounds = (0, 43);
    source.participations[0].byte_bounds = (0, 2_200);
    same_session_1.raw_source = same_session_0.raw_source.clone();
    let plan = stitched(plan_stitch(
        "g",
        stitch_inputs(),
        vec![same_session_1, same_session_0, snap_seg(2, 0, K, 30..=40)],
    ));
    assert_eq!(plan.segments[0].input.play_order, 2);
    assert_eq!(plan.segments[0].segment_id, 0);
    assert_eq!(plan.segments[1].input.play_order, 5);
    assert_eq!(plan.segments[1].input.in_session_ordinal, 0);
    assert_eq!(plan.segments[1].segment_id, 1);
    assert_eq!(plan.segments[2].segment_id, 2);
}

#[test]
fn empty_recovery_segment_retained_not_placed() {
    let mut recovery = snap_seg(1, 0, K, 0..=0);
    recovery.coverage.clear();
    recovery.observations.clear();
    let s0 = snap_seg(0, 0, K, 0..=10);
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![s0, recovery]));
    assert_eq!(plan.segments.len(), 2);
    let rec = plan
        .segments
        .iter()
        .find(|p| p.input.is_empty_recovery())
        .unwrap();
    assert!(rec.visible_spans.is_empty());
    assert!(rec.superseded_spans.is_empty());
    assert_eq!(rec.input.cutover(), None);
    assert_eq!(main_hours(&plan), (0..=10).collect::<Vec<_>>());
}

#[test]
fn empty_recovery_segment_cannot_claim_transition_hours_without_a_cutover() {
    let mut recovery = snap_seg(1, 0, K, 0..=0);
    recovery.coverage.clear();
    recovery.observations.clear();
    recovery.transition_coverage.insert(
        0,
        CoverageDisposition::Gap {
            reason: "unowned transition".into(),
            evidence_id: "transition-unowned-0".into(),
        },
    );
    match plan_stitch(
        "g",
        stitch_inputs(),
        vec![snap_seg(0, 0, K, 0..=10), recovery],
    ) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("without a cutover")),
        other => panic!("empty recovery transition must fail closed, got {other:?}"),
    }
}

#[test]
fn hostile_wide_hour_span_is_rejected_without_iterating_the_numeric_range() {
    let mut segment = snap_seg(0, 0, K, 0..=1);
    let last_disposition = segment.coverage.remove(&1).unwrap();
    let last_observation = segment.observations.remove(&1).unwrap();
    segment.coverage.insert(i64::MAX, last_disposition);
    segment.observations.insert(i64::MAX, last_observation);
    match plan_stitch("g", stitch_inputs(), vec![segment]) {
        StitchOutcome::Separate { reason } => assert!(reason.contains("missing coverage between")),
        other => panic!("wide sparse span must fail closed, got {other:?}"),
    }
}

#[test]
fn colliding_empty_recovery_segment_is_rejected_in_every_input_permutation() {
    let nonempty = snap_seg(0, 0, K, 0..=10);
    let mut empty = snap_seg(0, 0, K, 0..=0);
    empty.coverage.clear();
    empty.observations.clear();
    for inputs in [vec![nonempty.clone(), empty.clone()], vec![empty, nonempty]] {
        match plan_stitch("g", stitch_inputs(), inputs) {
            StitchOutcome::Separate { reason } => assert!(reason.contains("duplicate")),
            other => panic!("colliding retained empty segment must fail closed, got {other:?}"),
        }
    }
}

#[test]
fn adjacent_different_failure_causes_do_not_coalesce() {
    let mut segment = snap_seg(0, 0, K, 0..=1);
    segment.coverage.insert(
        0,
        CoverageDisposition::CaptureFailed {
            reason: "cause-a".into(),
            evidence_id: "evidence-a".into(),
        },
    );
    segment.observations.get_mut(&0).unwrap()[0].observation_id = "evidence-a".into();
    segment.observations.get_mut(&0).unwrap()[0].disposition = segment.coverage[&0].clone();
    segment.coverage.insert(
        1,
        CoverageDisposition::CaptureFailed {
            reason: "cause-b".into(),
            evidence_id: "evidence-b".into(),
        },
    );
    segment.observations.get_mut(&1).unwrap()[0].observation_id = "evidence-b".into();
    segment.observations.get_mut(&1).unwrap()[0].disposition = segment.coverage[&1].clone();
    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![segment]));
    let index = plan.main_index_value();
    let coverage = index["coverage"].as_array().unwrap();
    assert_eq!(coverage.len(), 2);
    assert_eq!(coverage[0]["reason"], "cause-a");
    assert_eq!(coverage[1]["reason"], "cause-b");
}

#[test]
fn same_hour_success_then_failure_keeps_both_and_serializes_exact_owner() {
    let mut segment = snap_seg(0, 0, K, 100..=100);
    let later = CoverageDisposition::CaptureFailed {
        reason: "later-frame-failed".into(),
        evidence_id: "frame-end-seq-20".into(),
    };
    segment.coverage.insert(100, later.clone());
    let owner = segment.observation_owner().unwrap();
    segment
        .observations
        .get_mut(&100)
        .unwrap()
        .push(HourObservation {
            observation_id: "frame-end-seq-20".into(),
            owner,
            record_seq_bounds: (2, 3),
            byte_bounds: (100, 200),
            disposition: later,
        });
    extend_source_bounds(&mut segment, 3, 200);

    let mut ambiguous = segment.clone();
    ambiguous.observations.get_mut(&100).unwrap()[1].record_seq_bounds = (0, 1);
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![ambiguous]),
        StitchOutcome::Separate { .. }
    ));

    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![segment]));
    let row = &plan.segment_index_value(&plan.segments[0])["hours"][0];
    assert_eq!(row["observation_count"], 2);
    assert_eq!(row["observations"].as_array().unwrap().len(), 2);
    assert_eq!(row["selected_observation_id"], "frame-end-seq-20");
    assert_eq!(row["owner"]["play_order"], 0);
    assert_eq!(row["owner"]["session_nonce"], "nonce-0");
    assert_eq!(row["kind"], "capture_failed");
}

#[test]
fn same_hour_failure_then_success_keeps_both_and_selects_later_sequence() {
    let mut segment = snap_seg(0, 0, K, 100..=100);
    let snapshot = segment.coverage[&100].clone();
    let earlier = CoverageDisposition::CaptureFailed {
        reason: "earlier-frame-failed".into(),
        evidence_id: "frame-end-seq-10".into(),
    };
    let owner = segment.observation_owner().unwrap();
    segment.observations.insert(
        100,
        vec![
            HourObservation {
                observation_id: "frame-end-seq-10".into(),
                owner: owner.clone(),
                record_seq_bounds: (0, 1),
                byte_bounds: (0, 100),
                disposition: earlier,
            },
            HourObservation {
                observation_id: "frame-end-seq-20".into(),
                owner,
                record_seq_bounds: (2, 3),
                byte_bounds: (100, 200),
                disposition: snapshot,
            },
        ],
    );
    segment.observations.get_mut(&100).unwrap().reverse();
    extend_source_bounds(&mut segment, 3, 200);

    let plan = stitched(plan_stitch("g", stitch_inputs(), vec![segment]));
    let row = &plan.segment_index_value(&plan.segments[0])["hours"][0];
    assert_eq!(row["observation_count"], 2);
    assert_eq!(row["observations"].as_array().unwrap().len(), 2);
    assert_eq!(row["selected_observation_id"], "frame-end-seq-20");
    assert_eq!(row["kind"], "snapshot");
}

#[test]
fn one_process_can_reenter_campaign_under_distinct_participation_tickets() {
    let mut first_a = snap_seg(0, 0, K, 0..=10);
    let mut second_a = snap_seg(1, 1, K, 11..=20);
    for segment in [&mut first_a, &mut second_a] {
        segment.provenance.session_nonce = "one-attachment".into();
        segment.provenance.process_start_identity = "one-process-start".into();
        let ticket = segment.ticket.as_mut().unwrap();
        ticket.session_nonce = "one-attachment".into();
        ticket.process_start_identity = "one-process-start".into();
        segment.provenance.source_path = Some("sources/session-source.h4cr".into());
        segment.provenance.source_stored_digest = Some("11".repeat(32));
        segment.provenance.source_decoded_digest = Some("22".repeat(32));
        let owner = segment.observation_owner().unwrap();
        for observations in segment.observations.values_mut() {
            for observation in observations {
                observation.owner = owner.clone();
            }
        }
    }
    first_a.provenance.record_seq_bounds = Some((0, 21));
    first_a.provenance.byte_bounds = Some((0, 1_100));
    for observations in second_a.observations.values_mut() {
        for observation in observations {
            observation.record_seq_bounds.0 += 200;
            observation.record_seq_bounds.1 += 200;
            observation.byte_bounds.0 += 2_100;
            observation.byte_bounds.1 += 2_100;
        }
    }
    second_a.provenance.record_seq_bounds = Some((200, 221));
    second_a.provenance.byte_bounds = Some((2_100, 3_200));

    let foreign_ticket = AppendTicket {
        campaign_key: b"campaign-B".to_vec(),
        play_order: 0,
        session_nonce: "one-attachment".into(),
        process_start_identity: "one-process-start".into(),
    };
    let shared_source = RawSourceDescriptor {
        session_nonce: "one-attachment".into(),
        process_start_identity: "one-process-start".into(),
        source_path: "sources/session-source.h4cr".into(),
        stored_digest: "11".repeat(32),
        decoded_digest: "22".repeat(32),
        record_seq_bounds: (0, 299),
        byte_bounds: (0, 3_200),
        storage: RawSourceStorage::Standalone,
        participations: vec![
            SourceParticipation {
                ticket: first_a.ticket.clone().unwrap(),
                scheme_major: 1,
                identity_evidence_digest: [0; 32],
                record_seq_bounds: (0, 99),
                byte_bounds: (0, 1_100),
            },
            SourceParticipation {
                ticket: foreign_ticket,
                scheme_major: 1,
                identity_evidence_digest: [9; 32],
                record_seq_bounds: (100, 199),
                byte_bounds: (1_100, 2_100),
            },
            SourceParticipation {
                ticket: second_a.ticket.clone().unwrap(),
                scheme_major: 1,
                identity_evidence_digest: [0; 32],
                record_seq_bounds: (200, 299),
                byte_bounds: (2_100, 3_200),
            },
        ],
    };
    first_a.raw_source = Some(shared_source.clone());
    second_a.raw_source = Some(shared_source);

    assert!(matches!(
        plan_stitch(
            "g",
            stitch_inputs(),
            vec![first_a.clone(), second_a.clone()]
        ),
        StitchOutcome::Stitched(_)
    ));

    let mut embedded_first = first_a;
    let mut embedded_second = second_a;
    for segment in [&mut embedded_first, &mut embedded_second] {
        segment.raw_source.as_mut().unwrap().storage = RawSourceStorage::EmbeddedCampaign {
            campaign_key: K.to_vec(),
        };
    }
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![embedded_first, embedded_second]),
        StitchOutcome::Separate { .. }
    ));

    let path_owner = snap_seg(0, 0, K, 0..=10);
    let mut path_alias = snap_seg(1, 0, K, 11..=20);
    let aliased_path = path_owner.raw_source.as_ref().unwrap().source_path.clone();
    path_alias.raw_source.as_mut().unwrap().source_path = aliased_path.clone();
    path_alias.provenance.source_path = Some(aliased_path);
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![path_owner, path_alias]),
        StitchOutcome::Separate { .. }
    ));

    let extent_owner = snap_seg(0, 0, K, 0..=10);
    let mut extent_alias = snap_seg(0, 1, K, 5..=15);
    extent_alias.raw_source = extent_owner.raw_source.clone();
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![extent_owner, extent_alias]),
        StitchOutcome::Separate { .. }
    ));
}

#[test]
fn windows_source_path_aliases_share_one_ownership_ledger() {
    for alias in [
        "c:/replay/s.h4cr",
        r"\\?\C:\Replay\S.h4cr",
        r"C:\Replay\redundant\..\S.h4cr",
    ] {
        let mut owner = snap_seg(0, 0, K, 0..=1);
        owner.raw_source.as_mut().unwrap().source_path = r"C:\Replay\S.h4cr".into();
        owner.provenance.source_path = Some(r"C:\Replay\S.h4cr".into());
        let mut reobserved = snap_seg(1, 0, K, 0..=1);
        reobserved.raw_source.as_mut().unwrap().source_path = alias.into();
        reobserved.provenance.source_path = Some(alias.into());
        assert!(matches!(
            plan_stitch("g", stitch_inputs(), vec![owner, reobserved]),
            StitchOutcome::Separate { .. }
        ));
    }
    let mut first = snap_seg(0, 0, K, 0..=1);
    first.raw_source.as_mut().unwrap().source_path = r"C:\Replay\S.h4cr".into();
    first.provenance.source_path = Some(r"C:\Replay\S.h4cr".into());
    let mut distinct = snap_seg(1, 0, K, 0..=1);
    distinct.raw_source.as_mut().unwrap().source_path = r"C:\Replay\T.h4cr".into();
    distinct.provenance.source_path = Some(r"C:\Replay\T.h4cr".into());
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![first, distinct]),
        StitchOutcome::Stitched(_)
    ));
}

#[test]
fn parent_is_captured_before_each_cutover_not_from_final_main() {
    let plan = stitched(plan_stitch(
        "g",
        stitch_inputs(),
        vec![
            snap_seg(0, 0, K, 0..=100),
            snap_seg(1, 0, K, 80..=90),
            snap_seg(2, 0, K, 50..=60),
        ],
    ));
    assert_eq!(plan.segments[1].parent_segment, Some(0));
    assert_eq!(plan.segments[2].parent_segment, Some(0));
}

#[test]
fn mandatory_source_inputs_are_rejected_and_schema_moves_digest() {
    let mut missing_source = snap_seg(0, 0, K, 0..=1);
    missing_source.raw_source = None;
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![missing_source]),
        StitchOutcome::Separate { .. }
    ));

    let mut writer_a = stitch_inputs();
    writer_a.schema_version = "writer-schema-a".into();
    let mut segment_a = snap_seg(0, 0, K, 0..=1);
    segment_a.provenance.schema_version = Some("writer-schema-a".into());
    if let CompatibilityResolution::Proven { schema_version, .. } = &mut segment_a.compatibility {
        *schema_version = "writer-schema-a".into();
    }
    let a = stitched(plan_stitch("g", writer_a, vec![segment_a]));
    let mut writer_b = stitch_inputs();
    writer_b.schema_version = "writer-schema-b".into();
    let mut segment_b = snap_seg(0, 0, K, 0..=1);
    segment_b.provenance.schema_version = Some("writer-schema-b".into());
    if let CompatibilityResolution::Proven { schema_version, .. } = &mut segment_b.compatibility {
        *schema_version = "writer-schema-b".into();
    }
    let b = stitched(plan_stitch("g", writer_b, vec![segment_b]));
    assert_ne!(a.stitch_input_digest, b.stitch_input_digest);
    let graph = a.segments_value();
    assert!(graph["fold_version"].is_string());
    assert!(graph["canonical_writer_version"].is_string());

    let base = stitched(plan_stitch(
        "g",
        stitch_inputs(),
        vec![snap_seg(0, 0, K, 0..=1)],
    ));
    let mutations: [fn(&mut StitchRequiredInputs); 7] = [
        |inputs| inputs.fold_version.push_str("-changed"),
        |inputs| inputs.canonical_writer_version.push_str("-changed"),
        |inputs| inputs.packer_codec_version.push_str("-changed"),
        |inputs| inputs.created_utc.push_str("-changed"),
        |inputs| inputs.compression_method.push_str("-changed"),
        |inputs| {
            inputs
                .compression_parameters
                .insert("level".into(), "changed".into());
        },
        |inputs| inputs.entry_order.push_str("-changed"),
    ];
    for mutate in mutations {
        let mut inputs = stitch_inputs();
        mutate(&mut inputs);
        let changed = stitched(plan_stitch("g", inputs, vec![snap_seg(0, 0, K, 0..=1)]));
        assert_ne!(base.stitch_input_digest, changed.stitch_input_digest);
    }

    let missing: [fn(&mut StitchRequiredInputs); 8] = [
        |inputs| inputs.schema_version.clear(),
        |inputs| inputs.fold_version.clear(),
        |inputs| inputs.canonical_writer_version.clear(),
        |inputs| inputs.packer_codec_version.clear(),
        |inputs| inputs.created_utc.clear(),
        |inputs| inputs.compression_method.clear(),
        |inputs| inputs.compression_parameters.clear(),
        |inputs| inputs.entry_order.clear(),
    ];
    for omit in missing {
        let mut inputs = stitch_inputs();
        omit(&mut inputs);
        assert!(matches!(
            plan_stitch("g", inputs, vec![snap_seg(0, 0, K, 0..=1)]),
            StitchOutcome::Separate { .. }
        ));
    }

    let mut changed_source = snap_seg(0, 0, K, 0..=1);
    let digest = "ab".repeat(32);
    changed_source.provenance.source_stored_digest = Some(digest.clone());
    changed_source.raw_source.as_mut().unwrap().stored_digest = digest;
    let changed = stitched(plan_stitch("g", stitch_inputs(), vec![changed_source]));
    assert_ne!(base.stitch_input_digest, changed.stitch_input_digest);
}

#[test]
fn raw_source_participations_must_partition_the_complete_source_extent() {
    let mut leading_hole = snap_seg(0, 0, K, 0..=1);
    {
        let source = leading_hole.raw_source.as_mut().unwrap();
        source.record_seq_bounds.0 = 0;
        source.byte_bounds.0 = 0;
        source.participations[0].record_seq_bounds.0 = 1;
        source.participations[0].byte_bounds.0 = 1;
    }
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![leading_hole]),
        StitchOutcome::Separate { .. }
    ));

    let mut trailing_hole = snap_seg(0, 0, K, 0..=1);
    {
        let source = trailing_hole.raw_source.as_mut().unwrap();
        source.record_seq_bounds.1 += 1;
        source.byte_bounds.1 += 1;
    }
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![trailing_hole]),
        StitchOutcome::Separate { .. }
    ));
}

#[test]
fn digests_and_transition_evidence_use_canonical_grammars() {
    let mut invalid_compatibility_digest = snap_seg(0, 0, K, 0..=1);
    if let CompatibilityResolution::Proven {
        build_exe_sha256, ..
    } = &mut invalid_compatibility_digest.compatibility
    {
        *build_exe_sha256 = "AB".repeat(32);
    }
    invalid_compatibility_digest.provenance.build_exe_sha256 = Some("AB".repeat(32));
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![invalid_compatibility_digest]),
        StitchOutcome::Separate { .. }
    ));

    let mut mismatched_publication_schema = snap_seg(0, 0, K, 0..=1);
    if let CompatibilityResolution::Proven { schema_version, .. } =
        &mut mismatched_publication_schema.compatibility
    {
        *schema_version = "foreign-schema".into();
    }
    mismatched_publication_schema.provenance.schema_version = Some("foreign-schema".into());
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![mismatched_publication_schema]),
        StitchOutcome::Separate { .. }
    ));

    let mut uppercase_source_digest = snap_seg(0, 0, K, 0..=1);
    uppercase_source_digest
        .raw_source
        .as_mut()
        .unwrap()
        .stored_digest = "AB".repeat(32);
    uppercase_source_digest.provenance.source_stored_digest = Some("AB".repeat(32));
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![uppercase_source_digest]),
        StitchOutcome::Separate { .. }
    ));

    let mut bad_snapshot_digest = snap_seg(0, 0, K, 0..=0);
    bad_snapshot_digest
        .coverage
        .get_mut(&0)
        .unwrap()
        .clone_from(&CoverageDisposition::Snapshot {
            sha256: "not-a-digest".into(),
            date: Some("1936.1.1.0".into()),
        });
    let bad_disposition = bad_snapshot_digest.coverage.get(&0).unwrap().clone();
    bad_snapshot_digest.observations.get_mut(&0).unwrap()[0].disposition = bad_disposition;
    assert!(matches!(
        plan_stitch("g", stitch_inputs(), vec![bad_snapshot_digest]),
        StitchOutcome::Separate { .. }
    ));

    let mut bad_transition = snap_seg(1, 0, K, 20..=20);
    for hour in 11..20 {
        bad_transition.transition_coverage.insert(
            hour,
            CoverageDisposition::Gap {
                reason: "forward gap".into(),
                evidence_id: "bad evidence with spaces".into(),
            },
        );
    }
    assert!(matches!(
        plan_stitch(
            "g",
            stitch_inputs(),
            vec![snap_seg(0, 0, K, 0..=10), bad_transition],
        ),
        StitchOutcome::Separate { .. }
    ));
}
