//! `lane_wall_tests`, extracted verbatim from `attach_log.rs`.

use super::*;

// Gate test (lever #5): the scheduler-assignment REPORT must match what
// actually ran. Inject a fake domain set through the real
// spawn_gated/ReaderGate path and check lane assignment + wall
// accounting consistency.
#[test]
fn lane_assignment_reporting_matches_execution() {
    const PERMITS: usize = 3;
    const FAKE_DOMAINS: u16 = 12;
    let gate = ReaderGate::new(PERMITS);
    let tracker = FrameLaneTracker::new(Instant::now(), 0, 0);
    thread::scope(|scope| {
        let gref = Some(&gate);
        let tref = &tracker;
        let handles: Vec<_> = (0..FAKE_DOMAINS)
            .map(|i| {
                spawn_gated(scope, gref, tref, 300 + i, move || {
                    // Busy window so lanes are actually contended.
                    for _ in 0..20_000 {
                        std::hint::spin_loop();
                    }
                    i
                })
            })
            .collect();
        for (i, handle) in handles.into_iter().enumerate() {
            assert_eq!(handle.join().unwrap(), i as u16);
        }
    });
    let spans = tracker.take_spans();
    // Every injected domain reported exactly once.
    assert_eq!(spans.len(), FAKE_DOMAINS as usize);
    let mut tags: Vec<u16> = spans.iter().map(|s| s.tag).collect();
    tags.sort_unstable();
    assert_eq!(tags, (300..300 + FAKE_DOMAINS).collect::<Vec<_>>());
    // Lane ids are real permit slots.
    for span in &spans {
        assert!(
            (span.lane as usize) < PERMITS,
            "lane {} out of range",
            span.lane
        );
        assert!(span.end_us >= span.start_us);
    }
    // A lane is a mutual-exclusion slot: its spans must not overlap.
    for lane in 0..PERMITS as u16 {
        let mut lane_spans: Vec<&LaneSpan> = spans.iter().filter(|s| s.lane == lane).collect();
        lane_spans.sort_by_key(|s| s.start_us);
        for pair in lane_spans.windows(2) {
            assert!(
                pair[1].start_us >= pair[0].end_us,
                "lane {lane} spans overlap: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }
    // The summary must agree with a manual recount of the same spans.
    let summary = summarize_lane_spans(&spans);
    assert_eq!(summary.main_wall_us, 0);
    assert!(!summary.lanes.is_empty());
    assert!(summary.lanes.len() <= PERMITS);
    let manual_max_wall = summary.lanes.iter().map(|&(_, w, _)| w).max().unwrap();
    assert_eq!(summary.crit_wall_us, manual_max_wall);
    let crit_lane = summary.crit_lane.expect("crit lane exists");
    let manual_crit_busy: u64 = spans
        .iter()
        .filter(|s| s.lane == crit_lane)
        .map(|s| s.end_us - s.start_us)
        .sum();
    assert_eq!(summary.crit_busy_us, manual_crit_busy);
    assert_eq!(
        summary.crit_domains.iter().map(|&(_, b)| b).sum::<u64>(),
        manual_crit_busy,
        "critical-path attribution must cover the whole critical lane"
    );
    assert!(summary.crit_busy_us <= summary.crit_wall_us);
}

#[test]
fn summarize_lane_spans_hand_built_exact() {
    let spans = [
        LaneSpan {
            tag: 20,
            lane: 0,
            start_us: 0,
            end_us: 100,
        },
        LaneSpan {
            tag: 21,
            lane: 0,
            start_us: 120,
            end_us: 200,
        },
        LaneSpan {
            tag: 22,
            lane: 1,
            start_us: 10,
            end_us: 50,
        },
        LaneSpan {
            tag: 63,
            lane: LW_MAIN_LANE,
            start_us: 210,
            end_us: 250,
        },
    ];
    let summary = summarize_lane_spans(&spans);
    assert_eq!(summary.lanes, vec![(0, 200, 180), (1, 40, 40)]);
    assert_eq!(summary.main_wall_us, 40);
    assert_eq!(summary.crit_lane, Some(0));
    assert_eq!(summary.crit_wall_us, 200);
    assert_eq!(summary.crit_busy_us, 180);
    assert_eq!(summary.crit_domains, vec![(20, 100), (21, 80)]);
}

#[test]
fn summarize_lane_spans_tie_prefers_lowest_lane() {
    let spans = [
        LaneSpan {
            tag: 30,
            lane: 2,
            start_us: 0,
            end_us: 90,
        },
        LaneSpan {
            tag: 31,
            lane: 1,
            start_us: 0,
            end_us: 90,
        },
    ];
    let summary = summarize_lane_spans(&spans);
    assert_eq!(summary.crit_lane, Some(1));
    assert_eq!(summary.crit_wall_us, 90);
}

#[test]
fn summarize_lane_spans_empty_is_zero() {
    let summary = summarize_lane_spans(&[]);
    assert_eq!(summary, LaneWallFrameSummary::default());
}

#[test]
fn lw_unit_names_are_stable_and_formatted() {
    let names = lw_lane_unit_names(3);
    assert_eq!(names[0], "lw_l3_wall_us");
    assert_eq!(names[1], "lw_l3_busy_us");
    assert!(std::ptr::eq(names, lw_lane_unit_names(3)));
    assert_eq!(lw_domain_crit_unit_name(48), "lw_d48_crit_us");
    assert_eq!(
        lw_domain_crit_unit_name(LW_TAG_COUNTRY_BUNDLE),
        "lw_d200_crit_us"
    );
}

#[test]
fn lane_wall_probe_record_is_wellformed_probe_domain_perf() {
    let spans = [
        LaneSpan {
            tag: 48,
            lane: 0,
            start_us: 0,
            end_us: 800,
        },
        LaneSpan {
            tag: LW_TAG_COUNTRY_BUNDLE,
            lane: 1,
            start_us: 0,
            end_us: 300,
        },
        LaneSpan {
            tag: 63,
            lane: LW_MAIN_LANE,
            start_us: 810,
            end_us: 860,
        },
    ];
    let summary = summarize_lane_spans(&spans);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out, 7, 123, 5, 900, 125, Some(2), 130, 700, &summary,
    );
    let header =
        compact_contract::parse_compact_record_header(&out).expect("well-formed record header");
    assert_eq!(header.domain_id, RC_PROBE_DOMAIN_ID);
    assert_eq!(header.record_type, COMPACT_RECORD_TYPE_DOMAIN_PERF);
    assert_ne!(
        header.record_flags & compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        0
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    assert_eq!(payload.len(), header.payload_len);
    let text = String::from_utf8_lossy(payload);
    for name in [
        "lw_frames",
        "lw_worker_span_us",
        "lw_prewarm_us",
        "lw_hour_label_skew_abs",
        "lw_hour_label_skew_missing",
        "lw_crit_wall_us",
        "lw_crit_busy_us",
        "lw_lmain_wall_us",
        "lw_l0_wall_us",
        "lw_l0_busy_us",
        "lw_l1_wall_us",
        "lw_l1_busy_us",
        "lw_d48_crit_us",
        "lw_prefetch_dist_relation",
        "lw_prefetch_dist_candidates",
    ] {
        assert!(text.contains(name), "missing unit {name}");
    }
}

// Micro-cost proof (lever #5): run with
//   cargo test -p collector --release lane_wall_micro_cost -- --ignored --nocapture
// Budget: < 100 us/frame for the WHOLE added path (48 span records +
// summary + probe-record encode).
