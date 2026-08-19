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
