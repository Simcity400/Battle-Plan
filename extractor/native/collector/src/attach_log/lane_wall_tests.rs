use std::collections::BTreeSet;

use super::*;

#[test]
fn lane_assignment_reporting_matches_execution() {
    const PERMITS: usize = 3;
    const FAKE_DOMAINS: u16 = 12;
    let gate = ReaderGate::new(PERMITS);
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    thread::scope(|scope| {
        let gref = Some(&gate);
        let tref = &tracker;
        let handles: Vec<_> = (0..FAKE_DOMAINS)
            .map(|i| {
                spawn_gated(scope, gref, tref, 300 + i, move || {
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
    assert_eq!(spans.len(), FAKE_DOMAINS as usize);
    let mut tags: Vec<u16> = spans.iter().map(|s| s.tag).collect();
    tags.sort_unstable();
    assert_eq!(tags, (300..300 + FAKE_DOMAINS).collect::<Vec<_>>());
    for span in &spans {
        assert!(
            (span.lane as usize) < PERMITS,
            "lane {} out of range",
            span.lane
        );
        assert!(span.end_us >= span.start_us);
    }
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
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
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

#[test]
fn country_bundle_sub_reads_are_attributed_under_their_own_units() {
    let specs = compact_country_domain_specs();
    let spec_of = |id: u16| {
        *specs
            .iter()
            .find(|spec| spec.id == id)
            .expect("country spec")
    };
    let read = |spec_ids: &[u16], read_time_us: u64| CompactBundleRead {
        specs: spec_ids.iter().copied().map(spec_of).collect(),
        rows: Vec::new(),
        read_time_us,
        row_count: 0,
        status_flags: 0,
        worker_panic_reason: None,
        technology_ids: Arc::new(TechnologyIdDictionary::default()),
        published_records: None,
    };
    let country_lanes = country_lane_attribution(&[
        read(&[COMPACT_DOMAIN_FUEL], 4_100),
        read(&[COMPACT_DOMAIN_POLITICS], 100),
        read(&[COMPACT_DOMAIN_FOCUS, COMPACT_DOMAIN_TECH], 91_000),
        read(&[COMPACT_DOMAIN_POLITICS], 30),
        read(&[], 500_000),
    ]);
    assert_eq!(
        country_lanes,
        vec![
            (COMPACT_DOMAIN_FUEL, 4_100),
            (COMPACT_DOMAIN_FOCUS, 91_000),
            (COMPACT_DOMAIN_POLITICS, 130),
        ]
    );
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    assert!(
        tracker.take_spans().is_empty(),
        "a sub-read runs inside the bundle's own lane span and must not record \
         a second span on it"
    );

    assert_eq!(lw_country_lane_unit_name(12), "lw_cb12_us");
    assert!(std::ptr::eq(
        lw_country_lane_unit_name(12),
        lw_country_lane_unit_name(12)
    ));

    let mut summary = summarize_lane_spans(&[LaneSpan {
        tag: LW_TAG_COUNTRY_BUNDLE,
        lane: 0,
        start_us: 0,
        end_us: 95_000,
    }]);
    summary.country_lanes = country_lanes;
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let text =
        String::from_utf8_lossy(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..]).into_owned();
    for name in ["lw_cb10_us", "lw_cb12_us", "lw_cb14_us"] {
        assert!(text.contains(name), "missing unit {name}");
    }
    assert!(text.contains("lw_d200_crit_us"));
    for name in ["lw_d10_crit_us", "lw_d12_crit_us", "lw_d14_crit_us"] {
        assert!(!text.contains(name), "sub-read leaked into {name}");
    }
}

#[test]
fn the_frame_builder_takes_the_country_attribution_off_the_bundle_drain() {
    let source = super::tests::collector_source_lf();
    let call = "let country_lane_us = country_lane_attribution(&country_reads);";
    assert_eq!(
        source.matches(call).count(),
        1,
        "the frame builder must call the attribution exactly once"
    );
    let assign = "lane_wall.country_lanes = country_lane_us;";
    assert_eq!(
        source.matches(assign).count(),
        1,
        "the summary's country lanes must come from that call and nothing else"
    );
    let drain = "for read in country_reads.drain(..) {";
    assert_eq!(source.matches(drain).count(), 1);
    let call_at = source.find(call).expect("call site");
    let drain_at = source.find(drain).expect("drain site");
    assert!(call_at < drain_at, "the attribution must precede the drain");
    let between = &source[call_at + call.len()..drain_at];
    assert!(
        between.trim().is_empty(),
        "nothing may sit between the attribution and the drain it is taken \
         off; found: {between:?}"
    );
    assert!(source.find(assign).expect("assign site") > drain_at);
}

/// MUTATION RUN (`--release`), verbatim:
#[test]
fn the_frame_cpu_split_counts_every_gated_worker_and_its_own_cycles() {
    const PERMITS: usize = 2;
    const WORKERS: u16 = 6;
    let gate = ReaderGate::new(PERMITS);
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    thread::scope(|scope| {
        let gref = Some(&gate);
        let tref = &tracker;
        let handles: Vec<_> = (0..WORKERS)
            .map(|i| {
                spawn_gated(scope, gref, tref, 400 + i, move || {
                    let mut acc: u64 = 0;
                    for step in 0..2_000_000u64 {
                        acc = acc.wrapping_add(step ^ u64::from(i));
                    }
                    std::hint::black_box(acc);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    tracker.close_fanout_process_cycles();
    let split = tracker.cpu_split();
    assert_eq!(
        split.worker_threads,
        u64::from(WORKERS),
        "every gated worker must contribute to the split, or the bound it \
         supports covers fewer workers than the wire claims"
    );
    assert!(
        split.worker_cycles > 0,
        "the workers burned real CPU, so a zero worker term means the split is \
         charging our cycles to the game"
    );
    assert!(
        !split.process_cycles_missing,
        "the process cycle counter must be readable on a supported host"
    );
    assert!(
        split.process_cycles >= split.worker_cycles,
        "process cycles {} < worker cycles {}; the subtraction that yields the \
         game's own cycles is not defined",
        split.process_cycles,
        split.worker_cycles
    );
}

/// MUTATION RUN (`--release`), verbatim: `cpu_split` changed to return the
#[test]
fn a_frame_with_no_process_counter_contributes_to_neither_term() {
    for (label, prepare) in [
        (
            "the origin counter was refused",
            (|t: &mut FrameLaneTracker| {
                t.origin_process_cycles = 0;
                t.fanout_end_process_cycles.store(9_000, Ordering::Relaxed);
            }) as fn(&mut FrameLaneTracker),
        ),
        (
            "the fan-out was never closed",
            |t: &mut FrameLaneTracker| {
                t.origin_process_cycles = 1_000;
            },
        ),
        ("the counter went backwards", |t: &mut FrameLaneTracker| {
            t.origin_process_cycles = 9_000;
            t.fanout_end_process_cycles.store(1_000, Ordering::Relaxed);
        }),
    ] {
        let mut tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
        prepare(&mut tracker);
        tracker.observe_worker_cycles(4_242);
        let split = tracker.cpu_split();
        assert!(split.process_cycles_missing, "{label}");
        assert_eq!(split.process_cycles, 0, "{label}");
        assert_eq!(
            split.worker_cycles, 0,
            "{label}: an unmeasurable frame must contribute nothing to the \
             worker term either"
        );
        assert_eq!(split.worker_threads, 0, "{label}");
    }
}

/// MUTATION RUN (`--release`), verbatim: `cpu_split` reverted to sampling
#[test]
fn the_process_term_is_closed_at_the_fanout_not_at_probe_time() {
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    tracker.close_fanout_process_cycles();
    let closed_at = tracker.fanout_end_process_cycles.load(Ordering::Relaxed);
    assert_ne!(closed_at, 0, "the process counter must be readable");
    let before = process_cycle_counter().expect("readable");
    let mut acc: u64 = 0;
    for step in 0..20_000_000u64 {
        acc = acc.wrapping_add(step ^ 0x9e37_79b9);
    }
    std::hint::black_box(acc);
    let burn_cycles = process_cycle_counter().expect("readable") - before;
    assert!(burn_cycles > 0, "the burn must move the process counter");
    let split = tracker.cpu_split();
    assert!(
        !split.process_cycles_missing,
        "a closed fan-out with a readable counter is a measurement"
    );
    assert!(
        split.process_cycles < burn_cycles,
        "the process term grew by work done after the fan-out closed: \
         {} cycles reported, {burn_cycles} burned afterwards",
        split.process_cycles
    );
}

#[test]
fn the_cpu_split_units_reach_the_probe() {
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        None,
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        0,
        0,
        0,
        &OutsideBuildSpans::default(),
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit {
            process_cycles: 987_654_321,
            worker_cycles: 123_456_789,
            worker_threads: 11,
            process_cycles_missing: false,
        },
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let trailer = &payload[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN..];
    assert_eq!(
        &trailer[..4],
        compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC
    );
    let (units, _) = compact_contract::diagnostic_channel::decode_units(&trailer[6..])
        .expect("the probe's own trailer must decode");
    for (name, expected) in [
        ("lw_fanout_process_cycles", 987_654_321u64),
        ("lw_fanout_worker_cycles", 123_456_789),
        ("lw_fanout_worker_threads", 11),
        ("lw_fanout_cycles_missing", 0),
    ] {
        let unit = units
            .iter()
            .find(|unit| unit.name == name)
            .unwrap_or_else(|| panic!("missing unit {name}"));
        assert_eq!(unit.value, expected, "unit {name} carries the wrong value");
    }
}

/// `--release`. Observed: this test failed with
#[test]
fn every_outside_build_segment_reaches_the_probe_under_its_own_name() {
    let mut outside = OutsideBuildSpans::default();
    for (index, slot) in outside.us.iter_mut().enumerate() {
        *slot = 1_000 + index as u64 * 1_000;
    }
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &outside,
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let units = compact_perf_diagnostic_units(payload);
    for (code, name) in compact_contract::diagnostic_channel::OUTSIDE_BUILD_SEGMENTS {
        let wire_name =
            compact_contract::diagnostic_channel::OUTSIDE_BUILD_UNIT_NAMES[usize::from(*code)];
        let unit = units
            .iter()
            .find(|unit| unit.name == wire_name)
            .unwrap_or_else(|| panic!("{wire_name} missing from the probe record"));
        assert_eq!(
            unit.value,
            outside.us[usize::from(*code)],
            "segment {code} ({name}) carries {} on the wire but the accumulator \
             holds {}",
            unit.value,
            outside.us[usize::from(*code)]
        );
    }
}

#[test]
fn a_zero_outside_build_segment_is_stated_rather_than_omitted() {
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        92_000,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let units = compact_perf_diagnostic_units(payload);
    for name in compact_contract::diagnostic_channel::OUTSIDE_BUILD_UNIT_NAMES {
        let unit = units
            .iter()
            .find(|unit| &unit.name == name)
            .unwrap_or_else(|| panic!("a segment measured as zero must still be stated: {name}"));
        assert_eq!(unit.value, 0);
    }
}

/// plain assignment, and this test re-run against it in `--release` after the
#[test]
fn outside_build_segments_accumulate_across_iterations() {
    const PASSES: u64 = 64;
    let mut outside = OutsideBuildSpans::default();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1) {
        std::hint::spin_loop();
    }
    for _ in 0..PASSES {
        outside.add_since(OutsideBuildSegment::Identity, started);
    }
    let elapsed_after_us = duration_micros_u64(started.elapsed());
    let total = outside.us[OutsideBuildSegment::Identity as usize];
    assert!(
        total > elapsed_after_us,
        "add_since must ACCUMULATE across the iterations between two builds: \
         {PASSES} closes of one ageing span totalled {total} us, which is no \
         more than the {elapsed_after_us} us that span had aged by the end — \
         that is what assignment gives"
    );
    assert!(
        total >= PASSES * 1_000,
        "each of the {PASSES} closes covered a span already older than 1 ms, \
         so the accumulated total cannot be under {} us; it was {total}",
        PASSES * 1_000
    );
    for (index, value) in outside.us.iter().enumerate() {
        if index != OutsideBuildSegment::Identity as usize {
            assert_eq!(*value, 0, "segment {index} collected another's time");
        }
    }
}

#[test]
fn outside_build_segment_discriminants_match_the_contract() {
    const DECLARED: &[(OutsideBuildSegment, &str)] = &[
        (OutsideBuildSegment::Total, "total"),
        (OutsideBuildSegment::Wait, "wait"),
        (OutsideBuildSegment::Identity, "identity"),
        (OutsideBuildSegment::Prepoll, "prepoll"),
        (OutsideBuildSegment::Prepub, "prepub"),
        (OutsideBuildSegment::Complete, "complete"),
        (OutsideBuildSegment::DegNotice, "degnotice"),
        (OutsideBuildSegment::DegLog, "deglog"),
        (OutsideBuildSegment::OpenPub, "openpub"),
        (OutsideBuildSegment::A30Insert, "a30insert"),
        (OutsideBuildSegment::HookStage, "hookstage"),
        (OutsideBuildSegment::TailInsert, "tailinsert"),
        (OutsideBuildSegment::BuildTail, "buildtail_previous_frame"),
    ];
    assert_eq!(
        DECLARED.len(),
        compact_contract::diagnostic_channel::OUTSIDE_BUILD_SEGMENT_COUNT,
        "a segment was added to the contract without a discriminant, or the \
         other way round"
    );
    for (segment, expected) in DECLARED {
        let code = *segment as usize;
        let contract =
            compact_contract::diagnostic_channel::outside_build_segment_name(code as u16)
                .unwrap_or_else(|| {
                    panic!("{segment:?} has discriminant {code}, outside the contract")
                });
        assert_eq!(
            contract, *expected,
            "OutsideBuildSegment::{segment:?} has discriminant {code} but the \
             contract gives code {code} to {contract:?}"
        );
    }
}

#[test]
fn frame_tail_segment_discriminants_match_the_contract() {
    const DECLARED: &[(FrameTailSegment, &str)] = &[
        (FrameTailSegment::DerivedEncode, "derived_encode"),
        (FrameTailSegment::CountryPublish, "country_publish"),
        (FrameTailSegment::PublishEarly, "publish_early"),
        (FrameTailSegment::D81Publish, "d81_publish"),
        (FrameTailSegment::PublishUnits, "publish_units"),
        (FrameTailSegment::D47Publish, "d47_publish"),
        (FrameTailSegment::PublishLate, "publish_late"),
    ];
    assert_eq!(
        DECLARED.len(),
        compact_contract::diagnostic_channel::FRAME_TAIL_SEGMENT_COUNT,
        "a segment was added to the contract without a discriminant, or the \
         other way round"
    );
    for (segment, expected) in DECLARED {
        let code = *segment as usize;
        let contract = compact_contract::diagnostic_channel::frame_tail_segment_name(code as u16)
            .unwrap_or_else(|| panic!("{segment:?} has discriminant {code}, outside the contract"));
        assert_eq!(
            contract, *expected,
            "FrameTailSegment::{segment:?} has discriminant {code} but the \
             contract gives code {code} to {contract:?}"
        );
    }
}

/// Every tail segment reaches the wire under the name its code derives, and
/// carries ITS OWN accumulator slot.
/// THE FRAME REPORTS WHETHER ITS TAIL WAS MEASURED AT ALL, and a value that
/// never passed through the marks says so on the wire.
///
/// This is what ended four rounds of source pins. Seven segment values cannot
/// distinguish a filled accumulator from a fresh one -- both are seven numbers,
/// and a never-marked one publishes seven honest zeros with every count, every
/// total and the host's whole population check intact, putting the entire
/// serial tail into "unattributed" under `valid:true`. Pins over this file's
/// source tried to prove the right value reaches the emitter, and rounds 23, 24
/// and 25 each defeated the previous round's pin with a compile-valid edit: a
/// decoy binding, a function-pointer alias beside a dead call, a module-level
/// `static frame_tail` shadowing the build-local accumulator. Text matching
/// cannot resolve names.
///
/// So the property is carried by the value instead of asserted about the text,
/// and it no longer matters HOW a wrong value arrives.
///
/// kills: emitting a constant in place of `frame_tail.marks()`.
/// kills: setting the mark bit anywhere other than in `close_at`, e.g. at
///   construction -- the defaulted half below then reports complete.
/// kills: dropping the unit, which the host's per-record census requires.
/// does NOT kill: closing one segment TWICE, which is an eighth boundary
///   rather than an unmeasured one; the source whitelist over the seven marks
///   is what catches that, and it is a different defect.
#[test]
fn a_tail_that_was_never_measured_says_so_on_the_wire() {
    let emit = |tail: &FrameTailSegments| {
        let summary = summarize_lane_spans(&[]);
        let mut out = Vec::new();
        append_compact_lane_wall_probe_record(
            &mut out,
            7,
            123,
            5,
            900,
            125,
            Some(2),
            FrameHourAge::default(),
            &[],
            &BTreeSet::new(),
            MapTopologyShareCounts::default(),
            130,
            700,
            0,
            &OutsideBuildSpans::default(),
            61_234,
            tail,
            &summary,
            FrameCpuSplit::default(),
        );
        let payload = out[compact_contract::COMPACT_RECORD_HEADER_LEN..].to_vec();
        compact_perf_diagnostic_units(&payload)
            .iter()
            .find(|unit| unit.name == compact_contract::diagnostic_channel::FRAME_TAIL_MARKS_UNIT)
            .map(|unit| unit.value)
    };

    // A TAIL THAT WAS ACTUALLY WALKED. Closed through the same `close_at` the
    // build body uses, one segment at a time, in contract order.
    let mut measured = FrameTailSegments::default();
    let mut mark = Instant::now();
    // Named one by one rather than derived from a code, so this list is the
    // same shape the build body writes and a segment added to the contract
    // without a mark shows up as a missing arm here.
    for segment in [
        FrameTailSegment::DerivedEncode,
        FrameTailSegment::CountryPublish,
        FrameTailSegment::PublishEarly,
        FrameTailSegment::D81Publish,
        FrameTailSegment::PublishUnits,
        FrameTailSegment::D47Publish,
        FrameTailSegment::PublishLate,
    ] {
        mark = measured.close_at(segment, mark, mark + Duration::from_micros(100));
    }
    assert_eq!(
        emit(&measured),
        Some(compact_contract::diagnostic_channel::FRAME_TAIL_MARKS_COMPLETE),
        "a frame that closed all seven segments must report all seven marks"
    );

    // AND ONE THAT WAS NOT. `FrameTailSegments::default()` is what every
    // defeated mutation put in front of the emitter -- a default, a static, an
    // aliased call's argument. Its seven values are indistinguishable from a
    // quiet frame's; its marks are not.
    let unmeasured = FrameTailSegments::default();
    assert_eq!(
        unmeasured.us,
        [0; compact_contract::diagnostic_channel::FRAME_TAIL_SEGMENT_COUNT],
        "the premise: the values alone say nothing"
    );
    assert_eq!(
        emit(&unmeasured),
        Some(0),
        "and a tail that was never closed must report no marks, so the host can \
         refuse the decomposition by name instead of publishing seven balanced \
         zeros"
    );
}

///
/// The values are distinct per slot on purpose: an off-by-one in the
/// `FRAME_TAIL_UNIT_NAMES` zip would still emit seven units of the right
/// names, and only a distinct value per slot catches it.
#[test]
fn every_frame_tail_segment_reaches_the_probe_under_its_own_name() {
    let mut tail = FrameTailSegments::default();
    for (index, slot) in tail.us.iter_mut().enumerate() {
        *slot = 3_000 + index as u64 * 1_100;
    }
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        61_234,
        &tail,
        &summary,
        FrameCpuSplit::default(),
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let units = compact_perf_diagnostic_units(payload);
    for (code, name) in compact_contract::diagnostic_channel::FRAME_TAIL_SEGMENTS {
        let wire_name =
            compact_contract::diagnostic_channel::FRAME_TAIL_UNIT_NAMES[usize::from(*code)];
        let unit = units
            .iter()
            .find(|unit| unit.name == wire_name)
            .unwrap_or_else(|| panic!("{wire_name} missing from the probe record"));
        assert_eq!(
            unit.value,
            tail.us[usize::from(*code)],
            "segment {code} ({name}) carries {} on the wire but the accumulator \
             holds {}",
            unit.value,
            tail.us[usize::from(*code)]
        );
    }
    let frame_time = units
        .iter()
        .find(|unit| unit.name == "lw_ft_us")
        .expect("lw_ft_us missing from the probe record");
    assert_eq!(
        frame_time.value, 61_234,
        "the frame's own time must reach the probe that counts lw_frames, or \
         the tail can only be differenced against a different population"
    );
}

/// A zero tail segment is STATED, not omitted — the same rule the outside-build
/// segments follow, and for the same reason: a consumer comparing two builds
/// has to be able to tell "this range cost nothing" from "this build does not
/// measure that range".
#[test]
fn a_zero_frame_tail_segment_is_stated_rather_than_omitted() {
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        0,
        &FrameTailSegments::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let units = compact_perf_diagnostic_units(payload);
    for name in compact_contract::diagnostic_channel::FRAME_TAIL_UNIT_NAMES {
        let unit = units
            .iter()
            .find(|unit| unit.name == *name)
            .unwrap_or_else(|| panic!("{name} was omitted when its value was zero"));
        assert_eq!(unit.value, 0);
    }
}

/// `close` charges the segment it is given and no other, and hands back a mark
/// the next segment starts from.
///
/// This is the property the whole partition rests on. A `close` that returned
/// the OLD mark would make every later segment include every earlier one, and
/// the totals would still look plausible — monotonically rising, summing to
/// something near the tail — which is exactly the failure a hand-read table
/// would not catch.
/// `close` charges the segment it is given and no other, and hands back the
/// mark the next segment starts from.
///
/// DETERMINISTIC: the instants are constructed, not spun for, so this asserts
/// on the accumulator's rules and never on the scheduler. An earlier draft
/// spun for 2 ms and then required the following segment to measure less than
/// that, which a single preemption between the two closes would have broken
/// while the code was correct.
///
/// This is the property the whole partition rests on. A `close` that returned
/// the OLD mark would make every later segment include every earlier one, and
/// the totals would still look plausible — monotonically rising, summing to
/// something near the tail — which is exactly the failure a hand-read table
/// would not catch.
#[test]
fn closing_a_frame_tail_segment_charges_only_that_segment_and_advances_the_mark() {
    let mut tail = FrameTailSegments::default();
    let origin = Instant::now();
    let at_two_ms = origin + Duration::from_millis(2);

    let returned = tail.close_at(FrameTailSegment::PublishEarly, origin, at_two_ms);
    assert_eq!(
        returned, at_two_ms,
        "close must hand back the instant it closed at, so the next segment \
         starts exactly where this one ended"
    );
    for (index, value) in tail.us.iter().enumerate() {
        if index == FrameTailSegment::PublishEarly as usize {
            assert_eq!(*value, 2_000, "the charged segment collected {value} us");
        } else {
            assert_eq!(*value, 0, "segment {index} collected another's time");
        }
    }

    // The returned mark is the NEW origin. Closing the next segment at +3 ms
    // must charge it 1 ms, not 3: a `close` that returned the old mark would
    // charge 3 here and the table would still look monotonic and plausible.
    let at_three_ms = origin + Duration::from_millis(3);
    tail.close_at(FrameTailSegment::D81Publish, returned, at_three_ms);
    assert_eq!(
        tail.us[FrameTailSegment::PublishEarly as usize],
        2_000,
        "closing a later segment rewrote an earlier one"
    );
    assert_eq!(
        tail.us[FrameTailSegment::D81Publish as usize],
        1_000,
        "the second segment was measured from a stale mark"
    );
}

/// A mark from the future charges zero rather than wrapping.
#[test]
fn a_frame_tail_close_before_its_own_mark_charges_zero() {
    let mut tail = FrameTailSegments::default();
    let origin = Instant::now();
    tail.close_at(
        FrameTailSegment::PublishLate,
        origin + Duration::from_millis(5),
        origin,
    );
    assert_eq!(tail.us[FrameTailSegment::PublishLate as usize], 0);
}

/// The segments PARTITION the tail: closing them in sequence from one origin
/// accounts for the whole span between the first mark and the last close,
/// exactly, with nothing double-counted and no internal hole.
#[test]
fn sequential_frame_tail_closes_partition_the_span_they_cover() {
    let mut tail = FrameTailSegments::default();
    let origin = Instant::now();
    let mut mark = origin;
    // Deliberately uneven, so a bug that divided the span evenly, or that
    // charged each segment the whole elapsed time from the origin, cannot pass.
    for (segment, offset_ms) in [
        (FrameTailSegment::DerivedEncode, 1),
        (FrameTailSegment::CountryPublish, 7),
        (FrameTailSegment::PublishEarly, 9),
        (FrameTailSegment::D81Publish, 40),
    ] {
        mark = tail.close_at(segment, mark, origin + Duration::from_millis(offset_ms));
    }
    assert_eq!(
        tail.us[FrameTailSegment::DerivedEncode as usize],
        1_000
    );
    assert_eq!(
        tail.us[FrameTailSegment::CountryPublish as usize],
        6_000
    );
    assert_eq!(tail.us[FrameTailSegment::PublishEarly as usize], 2_000);
    assert_eq!(tail.us[FrameTailSegment::D81Publish as usize], 31_000);
    assert_eq!(
        tail.total_us(),
        duration_micros_u64(mark.saturating_duration_since(origin)),
        "four adjacent segments must sum to exactly the span they cover"
    );
}

/// The SEVEN PRODUCTION MARKS, pinned against the collector's own source.
///
/// WHY THIS EXISTS. Every other test in this family exercises the accumulator
/// and the probe emitter, both of which keep working if production stops
/// CALLING them: replace all seven `frame_tail.close(..)` sites with nothing
/// and the frame publishes seven zero segments, every unit test still passes,
/// and the manifest reports a tail of 0 ms with the whole frame "unattributed".
/// The host's population check would catch a MISSING segment; it cannot catch
/// a segment that is faithfully emitted and always zero.
///
/// So this reads the production source and pins that all seven closes are
/// present, in contract order, and unconditional — at the function's own
/// four-space indentation, not nested inside an `if` that could skip them.
#[test]
fn the_seven_frame_tail_marks_are_present_in_contract_order_and_unconditional() {
    let source = include_str!("../attach_log.rs");
    const CALL: &str = "frame_tail.close(FrameTailSegment::";

    // EACH MARK WITH THE PRODUCTION LINE IT PRECEDES. Order alone is not the
    // property: a mark moved earlier or later still yields the same seven names
    // in the same order while the segment now brackets different code, and the
    // published `d81_publish` would be timing something that is not D81. The
    // landmark is the next production statement, which is what the segment
    // boundary MEANS (Codex review round 2, finding 4).
    // The third element is whether the returned mark MUST be kept. Segments 0-5
    // hand their close's instant to the next segment; only the last may discard
    // it, because there is no next segment. A close whose result is dropped
    // leaves `frame_tail_mark` where it was, so every following segment then
    // contains all the earlier ones -- while the seven names, their order and
    // their landmarks all still check out (Codex review round 4, finding 4).
    let expected: [(&str, &str, bool); 7] = [
        (
            "DerivedEncode",
            "let country_lane_us = country_lane_attribution(",
            true,
        ),
        (
            "CountryPublish",
            "if let Some(mut read) = frd_depth_read {",
            true,
        ),
        (
            "PublishEarly",
            "if let Some(mut read) = rail_replay_state_read {",
            true,
        ),
        ("D81Publish", "if let Some(read) = nuke_stockpile_read {", true),
        (
            "PublishUnits",
            "if let Some(read) = characters_leaders_cabinet_live_read {",
            true,
        ),
        (
            "D47Publish",
            "if let Some(mut read) = air_variant_reserve_read {",
            true,
        ),
        (
            "PublishLate",
            "let frame_time_us = duration_micros_u64(",
            false,
        ),
    ];

    let lines: Vec<&str> = source.lines().collect();
    let mut seen: Vec<(String, String, bool)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        // EXECUTABLE LINES ONLY. A commented-out `// frame_tail.close(..)`
        // otherwise satisfies a substring scan, so deleting a real call and
        // leaving the comment behind would pass.
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        // Exactly four spaces of indent and then code: the build body's own top
        // level. A mark nested inside an `if` or a loop indents further and is
        // rejected here, which is the "unconditional" half of the claim.
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        if rest.starts_with(' ') {
            continue;
        }
        if !rest.contains(CALL) {
            continue;
        }
        // THE WHOLE LINE MUST BE THE STATEMENT. A substring search accepts
        // `if false { frame_tail_mark = frame_tail.close(..); }`, which sits at
        // top-level indentation, supplies the substring and the expected next
        // landmark, and never executes (Codex review round 3, finding 5). Only
        // the two exact statement shapes are allowed, so any wrapper — a
        // conditional, a closure, a macro, a comment tail — fails.
        let keeps_mark = rest.starts_with("frame_tail_mark = ");
        let assigned = rest.strip_prefix("frame_tail_mark = ").unwrap_or(rest);
        let Some(at) = assigned.strip_prefix(CALL).map(|_| 0usize) else {
            panic!(
                "a frame-tail close is not a bare statement, so it may not run: {line:?}. \
                 Expected `frame_tail.close(FrameTailSegment::X, frame_tail_mark);` \
                 optionally preceded by `frame_tail_mark = `."
            );
        };
        let tail = &assigned[at + CALL.len()..];
        let end = tail
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(tail.len());
        assert_eq!(
            &tail[end..],
            ", frame_tail_mark);",
            "a frame-tail close does not take the running mark and end the \
             statement: {line:?}"
        );
        // The next line that is production code, which is the segment boundary.
        let next = lines[index + 1..]
            .iter()
            .map(|line| line.trim_start())
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap_or("");
        seen.push((tail[..end].to_string(), next.to_string(), keeps_mark));
    }

    let seen_pairs: Vec<(&str, &str, bool)> = seen
        .iter()
        .map(|(segment, next, keeps)| (segment.as_str(), next.as_str(), *keeps))
        .collect();
    assert_eq!(
        seen_pairs.len(),
        expected.len(),
        "expected exactly {} production marks, found {}: a duplicated or \
         deleted mark leaves a segment double-counted or always zero, which \
         every other test in this file would accept. Found: {seen_pairs:?}",
        expected.len(),
        seen_pairs.len()
    );
    for (index, ((segment, next, keeps), (want_segment, want_next, want_keeps))) in
        seen_pairs.iter().zip(expected.iter()).enumerate()
    {
        assert_eq!(
            segment, want_segment,
            "production mark {index} closes {segment}, contract order wants {want_segment}"
        );
        assert!(
            next.starts_with(want_next),
            "the mark closing {segment} is followed by {next:?}, not by {want_next:?}: \
             the mark has moved, so this segment now brackets different production code \
             than its name claims"
        );
        assert_eq!(
            keeps, want_keeps,
            "the mark closing {segment} {} the instant it closed at. Segments 0-5 must \
             assign it to frame_tail_mark so the next segment starts where this one \
             ended; a dropped mark makes every later segment contain this one, while the \
             names, the order and the landmarks all still check out.",
            if *keeps { "keeps" } else { "discards" }
        );
    }
    assert_eq!(
        expected.len(),
        compact_contract::diagnostic_channel::FRAME_TAIL_SEGMENT_COUNT,
        "a segment was added to the contract without a production mark"
    );

    // `close_at` is the TEST seam. Production must go through `close`, or the
    // clock could be supplied by the caller and the segments would measure
    // whatever that caller chose.
    for line in &lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        assert!(
            !trimmed.starts_with("frame_tail.close_at(")
                && !trimmed.contains("= frame_tail.close_at("),
            "production calls close_at, which takes its clock from the caller: {line:?}"
        );
    }
}

/// `close` reads a real clock, rather than reusing the mark it was handed.
///
/// `close_at` is what the deterministic tests above exercise; this is the one
/// property of `close` itself that matters, and it is the difference between a
/// working instrument and seven zeros. A lower bound against a mark placed 50 ms
/// in the past is monotonic-clock arithmetic, not a race: no scheduling can make
/// `Instant::now()` earlier than `now - 50ms`.
#[test]
fn a_frame_tail_close_reads_the_clock_rather_than_reusing_its_mark() {
    let mut tail = FrameTailSegments::default();
    let fifty_ms_ago = Instant::now() - Duration::from_millis(50);
    let returned = tail.close(FrameTailSegment::CountryPublish, fifty_ms_ago);
    assert!(
        tail.us[FrameTailSegment::CountryPublish as usize] >= 50_000,
        "close charged {} us against a mark 50 ms in the past; it is not \
         reading the clock",
        tail.us[FrameTailSegment::CountryPublish as usize]
    );
    // AND IT HANDS BACK THE CLOCK READING, not the mark. `close` is a wrapper
    // over `close_at`, and a wrapper that charged the right segment and then
    // returned `mark` would pass every other test in this file — the
    // deterministic ones call `close_at` directly, and the charge assertion
    // above is satisfied by the correct slot arithmetic. Production would then
    // never advance `frame_tail_mark` and every segment would contain all the
    // earlier ones (Codex review round 5, finding 5).
    assert!(
        returned.saturating_duration_since(fifty_ms_ago) >= Duration::from_millis(50),
        "close returned its own mark rather than the instant it closed at"
    );
}

/// Between its initialisation and its last mark, the build body may name the
/// accumulator ONLY on the seven bare close statements -- no more, no fewer,
/// nothing else -- and may not exit early between them.
///
/// The ordering test above pins WHICH seven statements exist, in what order,
/// against what landmarks. It cannot see anything indented past four spaces.
/// This one pins that NOTHING ELSE reaches the accumulator at any indent: a
/// direct `frame_tail.us[..] = ..` write would charge a segment whatever it
/// liked, an eighth close nested inside a branch would charge one twice, an
/// alias would do either -- and every ordering and landmark check would still
/// pass (Codex review rounds 5 finding 5, 6 finding 3, 7 finding 1). A
/// `return` or top-level `?;` between the marks would instead skip every
/// remaining close and publish a short tail on that frame.
#[test]
fn the_frame_tail_accumulator_is_reached_only_through_the_seven_unconditional_marks() {
    let source = include_str!("../attach_log.rs");
    let lines: Vec<&str> = source.lines().collect();
    let production = |line: &str| {
        let trimmed = line.trim_start();
        !trimmed.is_empty() && !trimmed.starts_with("//")
    };

    let first = lines
        .iter()
        .position(|line| line.contains("let mut frame_tail = FrameTailSegments::default();"))
        .expect("the tail accumulator is created in the build body");
    let last = lines
        .iter()
        .rposition(|line| {
            production(line) && line.contains("frame_tail.close(FrameTailSegment::PublishLate")
        })
        .expect("the last mark");
    assert!(first < last);

    // EVERY MENTION OF THE ACCUMULATOR, AT ANY INDENT. The earlier version of
    // this test enumerated the evasions it had already seen -- a direct
    // `frame_tail.us[..]` write, a re-assignment, a `return` -- and a mutation
    // it had not seen slipped between them: an EXTRA close, nested one level,
    // adds an eighth boundary and charges a segment twice (or moves that
    // segment's semantic boundary) while all seven top-level statements and
    // their landmarks still check out, so the host can still publish
    // `valid:true` (Codex review round 7, finding 1). The ordering test above
    // skips anything indented past four spaces, so it saw nothing either.
    //
    // So this stopped enumerating evasions and became a WHITELIST. Between the
    // accumulator's initialisation and the last mark, the identifier
    // `frame_tail` may appear ONLY on its initialisation line and on the seven
    // bare top-level close statements -- which is exactly what production
    // contains today, so the rule costs nothing and cannot be evaded by
    // indenting, aliasing, or adding a shape nobody thought to name.
    //
    // kills: a close wrapped in `if condition { .. }` and indented one level
    //   -- an eighth executable close. The line names the accumulator and is
    //   indented past four spaces, so the "nested or unindented" arm rejects
    //   it; written at top level instead, the close count reads 8 and the
    //   count assertion below rejects it.
    // kills: `let alias = &mut frame_tail;` followed by `alias.us[0] = 0;` --
    //   the alias line names the accumulator and is not one of the eight
    //   permitted shapes.
    // kills: a direct `frame_tail.us[3] = 0;` write and a re-assignment
    //   `frame_tail = FrameTailSegments::default();` -- the two the previous
    //   version named, now by this one rule rather than two special cases.
    // does NOT kill: a change to the BODY of `close`, which the deterministic
    //   `close_at` tests and the real-clock test cover, nor a mark moved to a
    //   different landmark, which the ordering test covers.
    fn is_ident_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_'
    }
    // Word-boundary, so `frame_tail_mark` -- a DIFFERENT binding, and the one
    // every close both reads and writes -- is not mistaken for the accumulator.
    let mentions_accumulator = |line: &str| {
        const NAME: &str = "frame_tail";
        let bytes = line.as_bytes();
        let mut from = 0usize;
        while let Some(hit) = line[from..].find(NAME) {
            let start = from + hit;
            let end = start + NAME.len();
            let before = start == 0 || !is_ident_byte(bytes[start - 1]);
            let after = end == bytes.len() || !is_ident_byte(bytes[end]);
            if before && after {
                return true;
            }
            from = end;
        }
        false
    };

    let mut initialisations = 0usize;
    let mut closes = 0usize;
    for (offset, line) in lines[first..=last].iter().enumerate() {
        if !production(line) {
            continue;
        }
        let number = first + offset + 1;
        let trimmed = line.trim_start();
        // NO `return` AT ANY INDENT, and no top-level `?`. An earlier version
        // of this test looked only at four-space indentation, so
        // `if condition { return Err(..); }` nested one level passed while
        // exiting the build and publishing a short tail (Codex round 6,
        // finding 3). The build body between the marks contains zero `return`
        // statements today at any depth, so the blanket rule costs nothing and
        // cannot be evaded by indenting.
        assert!(
            !trimmed.starts_with("return ") && !trimmed.starts_with("return;"),
            "line {number} can exit the build between the first and last \
             frame-tail mark, so a frame taking it publishes a short tail: {line:?}"
        );
        if let Some(rest) = line.strip_prefix("    ") {
            if !rest.starts_with(' ') {
                assert!(
                    !rest.trim_end().ends_with("?;"),
                    "line {number} is a top-level `?` between the first and last \
                     frame-tail mark, so a frame taking it publishes a short tail: {line:?}"
                );
            }
        }

        if !mentions_accumulator(line) {
            continue;
        }
        if number == first + 1 {
            assert_eq!(
                trimmed, "let mut frame_tail = FrameTailSegments::default();",
                "line {number} is where the accumulator is created, and it is \
                 not the plain initialisation: {line:?}"
            );
            initialisations += 1;
            continue;
        }
        let Some(rest) = line
            .strip_prefix("    ")
            .filter(|rest| !rest.starts_with(' '))
        else {
            panic!(
                "line {number} reaches the frame-tail accumulator from a nested \
                 or unindented position, so it is neither the initialisation nor \
                 one of the seven unconditional marks. A nested close adds an \
                 eighth boundary and charges a segment twice while the ordering \
                 test, which ignores anything indented past four spaces, still \
                 passes: {line:?}"
            );
        };
        let statement = rest.trim_end();
        let body = statement
            .strip_prefix("frame_tail_mark = ")
            .unwrap_or(statement);
        assert!(
            body.starts_with("frame_tail.close(FrameTailSegment::")
                && body.ends_with(", frame_tail_mark);"),
            "line {number} names the frame-tail accumulator but is not one of \
             the seven bare close statements, so it can charge a segment \
             whatever it likes while every ordering and landmark check still \
             passes: {line:?}"
        );
        closes += 1;
    }
    assert_eq!(
        initialisations, 1,
        "the accumulator must be created exactly once, on the first line of the \
         pinned range"
    );
    assert_eq!(
        closes,
        compact_contract::diagnostic_channel::FRAME_TAIL_SEGMENT_COUNT,
        "the pinned range must hold exactly one close per contract segment. An \
         extra close adds a boundary and charges a segment twice; a missing one \
         leaves a segment always zero. Found {closes}."
    );

    // WHERE THE HANDOFF IS PROVED, AND WHY NOT HERE.
    //
    // Everything above pins how the seven segments are FILLED. Whether the
    // filled accumulator is what gets PUBLISHED was pinned here for three
    // rounds, and each version fell to a compile-valid edit the next round
    // found: a decoy `let _pin_only = (&frame_tail,);` beside a defaulted call
    // (round 23), a function-pointer alias beside a dead one (round 24), a
    // module-level `static frame_tail` and a forwarding function defined after
    // the build body (round 25). Text matching cannot resolve names, and every
    // one of those pins reported coverage it did not have -- which is worse
    // than none, because a vacuous pin is green whichever way production goes.
    //
    // The property now travels on the wire instead. `FrameTailSegments` records
    // which segments it was actually given, the frame publishes that as
    // `lw_ft_marks`, and the host refuses a decomposition whose marks are not
    // complete -- so it no longer matters HOW a wrong value reaches the
    // emitter. See `a_tail_that_was_never_measured_says_so_on_the_wire` here
    // and `a_frame_that_never_closed_its_tail_segments_is_refused_not_read_as_zeros`
    // in the host.
}

//   cargo test -p collector --release lane_wall_micro_cost -- --ignored --nocapture

/// plain `-`. Observed under `--release`, which is what the gate runs: this
#[test]
fn a_build_tail_shorter_than_its_own_frame_time_records_zero() {
    assert_eq!(
        outside_build_tail_us(90_000, 93_000),
        0,
        "a build whose wall time reads shorter than the frame_time_us it \
         returned must record zero, never a wrapped u64"
    );
    assert_eq!(outside_build_tail_us(94_500, 93_000), 1_500);
    assert_eq!(outside_build_tail_us(0, 0), 0);

    let mut outside = OutsideBuildSpans::default();
    outside.add_us(
        OutsideBuildSegment::BuildTail,
        outside_build_tail_us(90_000, 93_000),
    );
    outside.add_us(
        OutsideBuildSegment::BuildTail,
        outside_build_tail_us(94_500, 93_000),
    );
    assert_eq!(outside.us[OutsideBuildSegment::BuildTail as usize], 1_500);
}

//   cargo test -p collector --release outside_build_preboundary_cost \
#[test]
#[ignore]
fn outside_build_preboundary_cost() {
    const HOUR: i32 = 61_000_000;
    const DATA_PAYLOADS: &[usize] = &[
        953_474, 439_056, 418_872, 414_556, 396_512, 383_408, 245_973, 231_936, 163_984, 158_500,
        146_437, 89_981,
    ];

    let records = match std::env::var("BATTLE_PLAN_LANE_AE_FRAME_BLOB").or_else(|_| std::env::var("HOI4_LANE_AE_FRAME_BLOB")) {
        Ok(path) => {
            let bytes = std::fs::read(&path).expect("frame blob");
            println!("frame blob {path}: {} bytes", bytes.len());
            bytes
        }
        Err(_) => {
            let mut out: Vec<u8> = Vec::new();
            let mut sequence = 0u64;
            for index in 0..51u16 {
                let mut payload = vec![0u8; 64];
                if index < 13 {
                    payload[32..36].copy_from_slice(&0x0000_0002u32.to_le_bytes());
                }
                append_compact_record(
                    &mut out,
                    sequence,
                    HOUR,
                    10 + index,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    0,
                    &payload,
                );
                sequence += 1;
            }
            let large: usize = DATA_PAYLOADS.iter().sum();
            let remainder = 4_586_802usize.saturating_sub(large) / 36;
            for (index, len) in DATA_PAYLOADS
                .iter()
                .copied()
                .chain(std::iter::repeat(remainder).take(36))
                .enumerate()
            {
                append_compact_record(
                    &mut out,
                    sequence,
                    HOUR,
                    10 + index as u16,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    0,
                    &vec![0x5Au8; len],
                );
                sequence += 1;
            }
            for index in 0..20u16 {
                append_compact_record(
                    &mut out,
                    sequence,
                    HOUR,
                    10 + index,
                    compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
                    0,
                    &vec![0u8; 48],
                );
                sequence += 1;
            }
            append_compact_record(
                &mut out,
                sequence,
                HOUR,
                compact_contract::COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                0,
                &vec![0u8; 64],
            );
            out
        }
    };

    let mut count = 0usize;
    let mut offset = 0usize;
    let mut last_sequence = 0u64;
    let mut last_hour = 0i32;
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        last_sequence = header.sequence;
        last_hour = header.game_hour_raw;
        offset += header.record_len();
        count += 1;
    }
    println!(
        "frame: {} bytes, {count} records, {:.1} KB/record",
        records.len(),
        records.len() as f64 / count as f64 / 1024.0
    );

    let outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    const REPS: u32 = 40;

    let started = Instant::now();
    let mut findings = 0usize;
    for _ in 0..REPS {
        findings += compact_frame_completeness_findings(&records, last_hour, &outcomes).len();
    }
    println!(
        "completeness walk: {:.1} us/frame ({findings} findings over {REPS} reps)",
        started.elapsed().as_secs_f64() * 1e6 / f64::from(REPS)
    );

    let started = Instant::now();
    for _ in 0..REPS {
        let buffer = records.clone();
        std::hint::black_box(&buffer);
    }
    let clone_us = started.elapsed().as_secs_f64() * 1e6 / f64::from(REPS);
    println!("bare {} byte clone: {clone_us:.1} us", records.len());

    let domains = vec![compact_contract::frame_degraded::FrameDegradedDomain {
        domain_id: 47,
        status_flags: 2,
        non_tolerated_flags: 2,
        predicate: compact_contract::diagnostic_channel::Predicate::undeclared(),
        authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
        reason: "bench".to_string(),
    }];

    let notice_len = compact_contract::COMPACT_RECORD_HEADER_LEN
        + compact_contract::frame_degraded::FrameDegradedRecordV2 {
            degraded_hour: last_hour,
            domains: domains.clone(),
        }
        .encode()
        .expect("bench notice encodes")
        .len();
    for (label, prepare) in [
        (
            "spare capacity",
            (|records: &Vec<u8>| {
                let mut buffer = Vec::with_capacity(records.len() + 65_536);
                buffer.extend_from_slice(records);
                buffer
            }) as fn(&Vec<u8>) -> Vec<u8>,
        ),
        ("shrunk to fit", |records: &Vec<u8>| {
            let mut buffer = records.clone();
            buffer.shrink_to_fit();
            buffer
        }),
    ] {
        let mut buffers: Vec<Vec<u8>> = (0..REPS).map(|_| prepare(&records)).collect();
        let mut arguments: Vec<Vec<compact_contract::frame_degraded::FrameDegradedDomain>> =
            (0..REPS).map(|_| domains.clone()).collect();
        let spare: Vec<usize> = buffers
            .iter()
            .map(|buffer| buffer.capacity() - buffer.len())
            .collect();
        let min_spare = spare.iter().copied().min().unwrap_or(0);
        let max_spare = spare.iter().copied().max().unwrap_or(0);
        let mut total = Duration::ZERO;
        for (buffer, argument) in buffers.iter_mut().zip(arguments.drain(..)) {
            let mut next = last_sequence + 1;
            let started = Instant::now();
            insert_frame_degraded_notice_before_frame_end(buffer, &mut next, last_hour, argument)
                .unwrap();
            total += started.elapsed();
        }
        let growth = if max_spare < notice_len {
            "MUST"
        } else if min_spare < notice_len {
            "may (mixed)"
        } else {
            "need not"
        };
        println!(
            "H4FD insert, {label}: {:.1} us/frame (insert only); spare capacity \
             {min_spare}..{max_spare} bytes vs a {notice_len}-byte insertion, so \
             splice {growth} grow",
            total.as_secs_f64() * 1e6 / f64::from(REPS),
        );
    }

    let started = Instant::now();
    for index in 0..REPS {
        record_attach_worker_error(
            "lane_ae_bench",
            &format!("hour {index} published with degraded domains: d47:0x00000002[bench]"),
        );
    }
    println!(
        "degraded-frame log append: {:.1} us/frame",
        started.elapsed().as_secs_f64() * 1e6 / f64::from(REPS)
    );
}
